//! Publish -> sweep -> pinnable, against a real database.
//!
//! This is the whole of G6's claim, and none of it is provable without a
//! database: every property here is a statement about rows in four tables at
//! once — `pricing_catalog_version_ref`, `pricing_read_model`,
//! `pricing_pin_frontier` and `pricing_outbox` — and about what a transaction
//! does to them together. A unit test over the projector could assert the calls
//! happen; it could not assert that a version reads complete, that a frontier
//! refuses to move, or that a second pass writes nothing.
//!
//! The harness is `sqlite_publish_commit.rs`'s, extended in the one way this
//! group needs: the registry double can be **told what to answer**. G5's double
//! answered `committed_version` with `Ok(None)` under the comment "G6's call,
//! and nothing in this suite makes it"; this one makes it.
//!
//! The G5 seam is not re-asserted here. `sqlite_publish_commit.rs` asserts it
//! on every success case and passes **unedited**, which is a stronger statement
//! than a copy of it would be.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bss_pricing::config::{JobsConfig, LimitsConfig};
use bss_pricing::domain::concurrency::RowVersion;
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::money::{CurrencyCode, MinorAmount};
use bss_pricing::domain::plan::PlanShapePatch;
use bss_pricing::domain::plan_shape::{
    BillingCycle, DescriptorSet, Frequency, PhaseKind, PlanPhase,
};
use bss_pricing::domain::price_record::PriceContent;
use bss_pricing::domain::price_row::{ModelKind, PriceRow};
use bss_pricing::domain::publish::{PlanPublishUnit, PublishAuthorization};
use bss_pricing::domain::read_model::SubjectRef;
use bss_pricing::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use bss_pricing::infra::fixture_gate::FixtureGate;
use bss_pricing::infra::jobs::readmodel_warm::{ReadModelWarmJob, SweepReport};
use bss_pricing::infra::publish::PublishService;
use bss_pricing::infra::storage::RepoError;
use bss_pricing::infra::storage::entity::{catalog_version_ref, outbox, plan, read_model};
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::{
    NewPlanDraft, NewPriceDraft, PendingVersionRow, PinFrontierRepo, PlanRepo, PlanShapeRepo,
    PriceRepo, catalog_version_ref_repo, plan_repo,
};
use bss_pricing_sdk::CatalogVersion;
use bss_pricing_sdk::catalog_version_registry::{
    CatalogVersionRegistryError, CatalogVersionRegistryV1, PendingVersionRef,
};
use chrono::{DateTime, TimeZone, Utc};
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order};
use sea_orm_migration::MigratorTrait;
use std::path::{Path, PathBuf};
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::{AccessScope, SecureEntityExt, SecureUpdateExt};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use toolkit_security::SecurityContext;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// The registry double, now with an answer for `committed_version`.
// ---------------------------------------------------------------------------

/// A registry that hands out one pending handle per `request_id` and can be
/// told what each handle later commits to.
///
/// `commits` maps a handle to the answers it gives, **in order**; the last
/// answer repeats once the script runs out. That is what lets a case say "not
/// yet, then version 5" and, for the invariant-breach case, "version 5, then
/// version 6" — a registry contradicting itself, which is the one answer the
/// finalize refuses.
#[derive(Default)]
struct RegistryDouble {
    issued: Mutex<HashMap<String, String>>,
    commits: Mutex<HashMap<String, Vec<Option<CatalogVersion>>>>,
    calls: Mutex<HashMap<String, usize>>,
    /// An outage: every `committed_version` call fails with this, which is
    /// **not** the same as `Unconfigured` and must not make the pass inert.
    outage: Mutex<Option<String>>,
    /// A **per-ref** outage — the condition D-163 clause (2) names, where one
    /// ref of a version cannot be resolved while its siblings can.
    unresolvable: Mutex<Vec<String>>,
}

impl RegistryDouble {
    /// Answer `pending_ref` with `version` from now on.
    fn commit(&self, pending_ref: &str, version: u64) {
        self.commits
            .lock()
            .expect("no panics in the double")
            .insert(
                pending_ref.to_owned(),
                vec![Some(CatalogVersion::new(version))],
            );
    }

    /// Fail every lookup — a configured registry that cannot be reached.
    fn fail_with(&self, error: &CatalogVersionRegistryError) {
        *self.outage.lock().expect("no panics in the double") = Some(error.to_string());
    }

    /// Fail lookups of **one** handle, leaving its siblings answerable.
    fn fail_handle(&self, pending_ref: &str) {
        self.unresolvable
            .lock()
            .expect("no panics in the double")
            .push(pending_ref.to_owned());
    }

    /// Let a previously unresolvable handle be answered again.
    fn clear_handle_failures(&self) {
        self.unresolvable
            .lock()
            .expect("no panics in the double")
            .clear();
    }

    /// Answer `pending_ref` with a scripted sequence, the last entry repeating.
    fn script(&self, pending_ref: &str, answers: Vec<Option<CatalogVersion>>) {
        self.commits
            .lock()
            .expect("no panics in the double")
            .insert(pending_ref.to_owned(), answers);
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
        pending_ref: &str,
    ) -> Result<Option<CatalogVersion>, CatalogVersionRegistryError> {
        if let Some(reason) = self.outage.lock().expect("no panics in the double").clone() {
            return Err(CatalogVersionRegistryError::Unreachable(reason));
        }
        if self
            .unresolvable
            .lock()
            .expect("no panics in the double")
            .iter()
            .any(|handle| handle == pending_ref)
        {
            return Err(CatalogVersionRegistryError::Unreachable(format!(
                "{pending_ref} is unresolvable"
            )));
        }
        let mut calls = self.calls.lock().expect("no panics in the double");
        let seen = calls.entry(pending_ref.to_owned()).or_insert(0);
        let index = *seen;
        *seen += 1;
        let commits = self.commits.lock().expect("no panics in the double");
        let Some(answers) = commits.get(pending_ref) else {
            return Ok(None);
        };
        Ok(answers
            .get(index)
            .copied()
            .unwrap_or_else(|| answers.last().copied().flatten()))
    }
}

// ---------------------------------------------------------------------------
// The harness.
// ---------------------------------------------------------------------------

const TENANT: Uuid = Uuid::from_u128(0x7e_11);
/// A second tenant, for the one property a per-tenant bound has and a
/// cross-tenant one cannot: that one tenant's saturated scan does not defer
/// another tenant's completions (D-163 clause 2).
///
/// **Above** [`TENANT`], because the tenant sweep order is ascending tenant id —
/// so this tenant is swept *after* the saturated one, which is the direction that
/// would fail under a cross-tenant page.
const OTHER_TENANT: Uuid = Uuid::from_u128(0x7e_22);
const ACTOR: Uuid = Uuid::from_u128(0xac_10);
const CORRELATION: Uuid = Uuid::from_u128(0xc0_11);

fn at(hour: u32) -> DateTime<Utc> {
    at_min(hour, 0)
}

/// Minute resolution, because two publishes of one pass must be tellable apart.
fn at_min(hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 3, hour, minute, 0).unwrap()
}

fn ctx_of(tenant: Uuid) -> SecurityContext {
    SecurityContext::builder()
        .subject_id(ACTOR)
        .subject_tenant_id(tenant)
        .build()
        .expect("a subject and a tenant are all a context needs")
}

fn committed_registry_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus/registry.toml")
}

struct Harness {
    plans: PlanRepo,
    shapes: PlanShapeRepo,
    prices: PriceRepo,
    publish: PublishService,
    frontier: PinFrontierRepo,
    job: ReadModelWarmJob,
    registry: Arc<RegistryDouble>,
    scope: AccessScope,
    provider: DBProvider<DbError>,
}

async fn harness() -> Harness {
    harness_with(JobsConfig::default()).await
}

async fn harness_with(jobs: JobsConfig) -> Harness {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    let provider = DBProvider::<DbError>::new(db);
    let registry = Arc::new(RegistryDouble::default());
    let publish = PublishService::new(
        provider.clone(),
        &LimitsConfig::default(),
        FixtureGate::load(&committed_registry_path()),
        Arc::clone(&registry) as Arc<dyn CatalogVersionRegistryV1>,
    );
    Harness {
        plans: PlanRepo::new(provider.clone()),
        shapes: PlanShapeRepo::new(provider.clone()),
        prices: PriceRepo::new(provider.clone()),
        publish,
        frontier: PinFrontierRepo::new(provider.clone()),
        job: ReadModelWarmJob::new(
            provider.clone(),
            Arc::clone(&registry) as Arc<dyn CatalogVersionRegistryV1>,
            jobs,
        ),
        registry,
        scope: AccessScope::for_tenant(TENANT),
        provider,
    }
}

fn plan_draft_of(tenant: Uuid, plan_id: PlanId, tier: &str) -> NewPlanDraft {
    NewPlanDraft {
        plan_id,
        tenant_id: tenant,
        created_by: ACTOR,
        created_at_utc: at(10),
        sku_id: Some(Uuid::from_u128(0x5_c1)),
        plan_tier: Some(tier.to_owned()),
        billing_cycle: Some(BillingCycle::Recurring),
        frequency: Some(Frequency::Monthly),
        plan_tier_override: false,
        purchase_min_qty: None,
        purchase_max_qty: None,
        invoice_grouping_key: None,
        available_from: None,
        available_to: None,
    }
}

fn flat_row() -> PriceContent {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(MinorAmount::new(9_900).expect("a non-negative amount"));
    PriceContent {
        row,
        tax_inclusive: false,
        billing_timing: Some("advance".to_owned()),
        rounding_policy_ref: Some("half_up".to_owned()),
        grandfather_until: None,
        supersedes_price_id: None,
    }
}

fn scope_key(plan_id: PlanId, phase: PhaseId) -> ScopeKey {
    ScopeKey::new(
        plan_id,
        CurrencyCode::new("EUR").expect("three letters"),
        Region::new("eu").expect("a non-blank region"),
        phase,
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("the class pairs with cohort none")
}

/// A plan the whole rule set passes, plus one flat recurring row.
async fn seed_publishable(h: &Harness, plan_id: PlanId, tier: &str) -> (u64, RowVersion) {
    seed_publishable_of(h, TENANT, plan_id, tier).await
}

async fn seed_publishable_of(
    h: &Harness,
    tenant: Uuid,
    plan_id: PlanId,
    tier: &str,
) -> (u64, RowVersion) {
    let scope = AccessScope::for_tenant(tenant);
    let phase = PhaseId::new(Uuid::new_v4());
    let created = h
        .plans
        .create_draft(&scope, plan_draft_of(tenant, plan_id, tier))
        .await
        .expect("create the draft");
    let after_phases = h
        .shapes
        .replace_phases(
            &scope,
            tenant,
            plan_id,
            created.revision,
            created.row_version,
            vec![PlanPhase {
                phase_id: phase,
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
    let after_descriptors = h
        .shapes
        .set_descriptor_set(
            &scope,
            tenant,
            plan_id,
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

    h.prices
        .create_draft(
            &scope,
            tenant,
            NewPriceDraft {
                price_id: Uuid::new_v4(),
                scope_key: scope_key(plan_id, phase),
                content: flat_row(),
                created_by: ACTOR,
                created_at_utc: at(10),
            },
        )
        .await
        .expect("author the price row");

    (created.revision, after_descriptors.row_version)
}

/// Publish a seeded plan at `now` and hand back the pending handle its commit
/// recorded.
///
/// **The instant is a parameter, and that is not decoration.** An earlier
/// version of this helper committed every publish at one literal instant, so
/// every `requested_at` in the store was identical — and a guard that compared
/// request instants therefore matched every row, which made a whole class of
/// frontier defect unexpressible by this suite. In production the instant is
/// the commit's own `now`. A fixture that cannot tell two publishes apart is a
/// fixture carrying the property the code is supposed to carry.
async fn publish(
    h: &Harness,
    plan_id: PlanId,
    revision: u64,
    version: RowVersion,
    now: DateTime<Utc>,
) -> String {
    publish_of(h, TENANT, plan_id, revision, version, now).await
}

async fn publish_of(
    h: &Harness,
    tenant: Uuid,
    plan_id: PlanId,
    revision: u64,
    version: RowVersion,
    now: DateTime<Utc>,
) -> String {
    let receipt = h
        .publish
        .commit(
            &ctx_of(tenant),
            &AccessScope::for_tenant(tenant),
            tenant,
            PlanPublishUnit::new(plan_id, revision),
            version,
            PublishAuthorization::auto_publishable(),
            ACTOR,
            CORRELATION,
            now,
        )
        .await
        .expect("the publish commits");
    receipt
        .version_ref()
        .pending_ref()
        .expect("a commit stamps a pending handle, never a version")
        .to_owned()
}

/// Seed a plan, publish it at `now`, and return `(plan_id, pending_ref)`.
async fn seed_and_publish_at(h: &Harness, tier: &str, now: DateTime<Utc>) -> (PlanId, String) {
    seed_and_publish_for(h, TENANT, tier, now).await
}

async fn seed_and_publish_for(
    h: &Harness,
    tenant: Uuid,
    tier: &str,
    now: DateTime<Utc>,
) -> (PlanId, String) {
    let plan_id = PlanId::new(Uuid::new_v4());
    let (revision, version) = seed_publishable_of(h, tenant, plan_id, tier).await;
    let pending = publish_of(h, tenant, plan_id, revision, version, now).await;
    (plan_id, pending)
}

/// Flip a plan's current revision `published -> retired`.
///
/// By statement rather than through a repository, because **retirement has no
/// writer in this gear**: it is a publish unit of its own (D-128) and the group
/// that lands it is not this one. The flip is one the `pricing_plan` trigger
/// whitelist explicitly sanctions, so this is the state a retired plan is in
/// and not a state reached around the guards.
async fn retire(h: &Harness, plan_id: PlanId) {
    let conn = h.provider.conn().expect("conn");
    let moved = plan::Entity::update_many()
        .secure()
        .scope_with(&h.scope)
        .col_expr(
            plan::Column::LifecycleState,
            sea_orm::sea_query::Expr::value("retired"),
        )
        .filter(
            Condition::all()
                .add(plan::Column::TenantId.eq(TENANT))
                .add(plan::Column::PlanId.eq(plan_id.get()))
                .add(plan::Column::LifecycleState.eq("published")),
        )
        .exec(&conn)
        .await
        .expect("the trigger whitelist sanctions published -> retired");
    assert_eq!(moved.rows_affected, 1, "one current revision was retired");
}

async fn sweep(h: &Harness, now: DateTime<Utc>) -> SweepReport {
    h.job.run(now).await.expect("the sweep pass runs")
}

// ---------------------------------------------------------------------------
// Reading the tables back.
// ---------------------------------------------------------------------------

async fn deltas(h: &Harness) -> Vec<read_model::Model> {
    let conn = h.provider.conn().expect("conn");
    read_model::Entity::find()
        .secure()
        .scope_with(&h.scope)
        .order_by(read_model::Column::CatalogVersion, Order::Asc)
        .all(&conn)
        .await
        .expect("read the read model")
}

async fn refs(h: &Harness) -> Vec<catalog_version_ref::Model> {
    let conn = h.provider.conn().expect("conn");
    catalog_version_ref::Entity::find()
        .secure()
        .scope_with(&h.scope)
        .order_by(catalog_version_ref::Column::PendingRef, Order::Asc)
        .all(&conn)
        .await
        .expect("read the version refs")
}

async fn degraded_events(h: &Harness) -> Vec<outbox::Model> {
    let conn = h.provider.conn().expect("conn");
    outbox::Entity::find()
        .secure()
        .scope_with(&h.scope)
        .filter(Condition::all().add(outbox::Column::EventName.eq("PlanPublishDegraded")))
        .all(&conn)
        .await
        .expect("read the outbox")
}

async fn frontier_version(h: &Harness) -> Option<u64> {
    frontier_version_of(h, TENANT).await
}

async fn frontier_version_of(h: &Harness, tenant: Uuid) -> Option<u64> {
    h.frontier
        .read(&AccessScope::for_tenant(tenant), tenant)
        .await
        .expect("read the frontier")
        .map(|frontier| frontier.catalog_version.get())
}

// ---------------------------------------------------------------------------
// 1. The path, end to end.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_committed_publish_becomes_pinnable_after_one_sweep() {
    // The seam closes. Before this group a successful commit left a pending
    // handle and nothing resolvable at all; the assertion at the end is the one
    // G7's e2e inherits, and the only read-side end-to-end statement it can
    // make.
    let h = harness().await;
    let (plan_id, pending) = seed_and_publish_at(&h, "gold", at_min(12, 0)).await;
    h.registry.commit(&pending, 1);

    let report = sweep(&h, at(13)).await;
    assert!(!report.inert);
    assert_eq!(report.versions_projected, 1);
    assert_eq!(report.subjects_projected, 1);
    assert_eq!(report.frontiers_advanced, 1);

    let rows = deltas(&h).await;
    assert_eq!(rows.len(), 1, "one subject, one delta");
    assert_eq!(rows[0].catalog_version, 1);
    assert_eq!(rows[0].subject_kind, "plan");
    assert_eq!(rows[0].subject_ref, plan_id.to_string());
    assert!(rows[0].warm_completed, "the row is written warm");
    assert_eq!(
        rows[0].warm_completed_at,
        Some(at(13)),
        "the marker and its instant move together"
    );
    assert_eq!(
        rows[0].payload.get("planId"),
        Some(&serde_json::json!(plan_id.get()))
    );

    let stored = refs(&h).await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].catalog_version, Some(1));
    assert_eq!(
        stored[0].committed_at,
        Some(at(13)),
        "the commit CHECK ties the version to its instant"
    );

    assert_eq!(
        frontier_version(&h).await,
        Some(1),
        "PinFrontierRepo::read now answers, which is what G7's e2e asserts"
    );
}

#[tokio::test]
async fn a_resolved_plan_subjects_delta_stamps_the_cross_boundary_marker() {
    // D-169 clause (1), against the store rather than against the renderer. The
    // marker is a launch-constant tenant-wide value on every resolved `plan`
    // subject row, and the field that used to sit beside it left the contract -
    // so what a version freezes here is machine-readable and derivable by nobody
    // but this gear, and the sentence a human is shown is PRD AC #66's.
    let h = harness().await;
    let (_, pending) = seed_and_publish_at(&h, "gold", at_min(12, 0)).await;
    h.registry.commit(&pending, 1);

    sweep(&h, at(13)).await;

    let rows = deltas(&h).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].payload.get("crossBoundaryChangePolicy"),
        Some(&serde_json::json!("cancel_plus_new")),
        "{:?}",
        rows[0].payload
    );
    assert!(
        !rows[0]
            .payload
            .to_string()
            .to_ascii_lowercase()
            .contains("warningtext"),
        "and no warning text, under any spelling: {:?}",
        rows[0].payload
    );
}

#[tokio::test]
async fn a_non_plan_subject_carries_neither_half_because_it_has_no_delta_at_all() {
    // The contract is per D-91's keying: `crossBoundaryChangePolicy` lives on a
    // `plan` subject row, and the other three kinds have no store in this gear -
    // so the projector refuses them by name rather than writing a delta with a
    // marker and nothing else in it.
    //
    // That refusal is what makes "no non-plan subject carries the marker" true by
    // construction rather than by a renderer remembering: `PlanSubjectDelta` is
    // the crate's only delta renderer.
    let h = harness().await;
    let conn = h.provider.conn().expect("conn");
    catalog_version_ref_repo::record_pending(
        &conn,
        &h.scope,
        PendingVersionRow::for_subject(
            TENANT,
            "pend-overlay".to_owned(),
            &SubjectRef::PriceOverlay(Uuid::new_v4()),
            Some(0),
            Some(LifecycleState::Published),
            at_min(12, 0),
        ),
    )
    .await
    .expect("record an overlay subject's ref");
    h.registry.commit("pend-overlay", 4);

    let report = sweep(&h, at(13)).await;

    assert_eq!(
        report.subjects_failed, 1,
        "the overlay subject is refused by name"
    );
    assert!(
        deltas(&h).await.is_empty(),
        "so there is no row for a marker to be on"
    );
}

// ---------------------------------------------------------------------------
// 2. Batching — the finding this group's migration amendment exists for.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn two_publishes_of_one_tenant_commit_into_one_version() {
    // D-47's normal case, and the one `uq_pricing_catalog_version_ref_version`
    // made physically impossible: several of a tenant's pending refs committing
    // into ONE version. Under that index the second finalize failed outright.
    let h = harness().await;
    let (plan_a, pending_a) = seed_and_publish_at(&h, "gold", at_min(12, 0)).await;
    let (plan_b, pending_b) = seed_and_publish_at(&h, "silver", at_min(12, 1)).await;
    h.registry.commit(&pending_a, 4);
    h.registry.commit(&pending_b, 4);

    let report = sweep(&h, at(13)).await;
    assert_eq!(report.versions_projected, 1, "one version, two subjects");
    assert_eq!(report.subjects_projected, 2);
    assert_eq!(report.frontiers_advanced, 1, "one advance, not two");

    let rows = deltas(&h).await;
    assert_eq!(rows.len(), 2);
    let mut subjects: Vec<&str> = rows.iter().map(|row| row.subject_ref.as_str()).collect();
    subjects.sort_unstable();
    let mut expected = vec![plan_a.to_string(), plan_b.to_string()];
    expected.sort();
    assert_eq!(subjects, expected);
    assert!(rows.iter().all(|row| row.catalog_version == 4));

    assert!(
        refs(&h)
            .await
            .iter()
            .all(|row| row.catalog_version == Some(4)),
        "both refs finalized at the same version"
    );
    assert_eq!(frontier_version(&h).await, Some(4));
}

// ---------------------------------------------------------------------------
// 3 + 4. The prefix, and the forward walk that keeps it from stranding.
// ---------------------------------------------------------------------------

/// A `V5` whose second subject cannot project yet, and a complete `V6`.
///
/// The incompleteness is a **per-subject projection fault**, which is the shape
/// incompleteness actually takes once batch atomicity is assumed: the registry
/// commits a batch as a unit, so a version above the frontier is short of
/// pin-eligibility because one of its subjects would not project, not because
/// one of its refs is unknown. Here the second subject names a plan whose
/// revision row is not there when the sweep arrives — and `seed_late_subject`
/// is what makes it repairable, so the walk case can continue from this one.
async fn seed_incomplete_five_and_complete_six(h: &Harness) -> PlanId {
    let (_, pending_a) = seed_and_publish_at(h, "gold", at_min(12, 0)).await;
    let late = PlanId::new(Uuid::new_v4());
    let pending_late = "pend-late".to_owned();
    let conn = h.provider.conn().expect("conn");
    catalog_version_ref_repo::record_pending(
        &conn,
        &h.scope,
        PendingVersionRow::for_subject(
            TENANT,
            pending_late.clone(),
            &SubjectRef::Plan(late.get()),
            Some(0),
            Some(LifecycleState::Published),
            at_min(12, 1),
        ),
    )
    .await
    .expect("record the second subject of V5");
    let (_, pending_c) = seed_and_publish_at(h, "bronze", at_min(12, 2)).await;

    h.registry.commit(&pending_a, 5);
    h.registry.commit(&pending_late, 5);
    h.registry.commit(&pending_c, 6);
    late
}

/// Give the late subject the published revision it was waiting for, without
/// minting a second ref for it.
async fn seed_late_subject(h: &Harness, plan_id: PlanId) {
    let (revision, version) = seed_publishable(h, plan_id, "silver").await;
    let (_, outcome) = h
        .provider
        .db()
        .in_transaction::<(), bss_pricing::infra::storage::RepoError, _>(move |txn| {
            let scope = AccessScope::for_tenant(TENANT);
            Box::pin(async move {
                plan_repo::publish_revision(txn, &scope, TENANT, plan_id, revision, version)
                    .await
                    .map(|_| ())
            })
        })
        .await;
    outcome.expect("the late subject's revision publishes");
}

#[tokio::test]
async fn a_complete_later_version_does_not_move_the_frontier_over_an_incomplete_earlier_one() {
    // D-114's prefix, which is the whole reason pin-eligibility is version-level
    // AND prefix-closed: without it a pin of V6 resolved the late plan at V4
    // and then, once its warm landed, at V5 - one pin, two contents.
    let h = harness().await;
    let _ = seed_incomplete_five_and_complete_six(&h).await;

    let report = sweep(&h, at(13)).await;
    assert_eq!(report.versions_projected, 2, "V5 and V6 both saw a pass");
    assert_eq!(
        report.subjects_failed, 1,
        "V5's second subject could not project"
    );
    assert_eq!(
        report.frontiers_advanced, 0,
        "V5 is incomplete, and V6 is not the frontier's next in order"
    );
    assert_eq!(
        frontier_version(&h).await,
        None,
        "a tenant whose first version is incomplete has nothing it may pin"
    );

    // V6 really is fully warm, which is what makes the refusal a prefix
    // decision rather than an incompleteness one.
    let rows = deltas(&h).await;
    assert!(rows.iter().any(|row| row.catalog_version == 6));
}

#[tokio::test]
async fn completing_the_earlier_version_walks_the_frontier_through_to_the_later_one() {
    // The D-136 divergence, pinned as behaviour. Read literally, the frontier
    // advances only in the transaction completing its NEXT version in order -
    // so when V5 finally completes the frontier reaches V5, and V6, already
    // complete, would never see another completion to advance on. The
    // implementation walks forward instead.
    let h = harness().await;
    let late = seed_incomplete_five_and_complete_six(&h).await;
    sweep(&h, at(13)).await;
    assert_eq!(frontier_version(&h).await, None);

    seed_late_subject(&h, late).await;
    let report = sweep(&h, at(14)).await;

    assert_eq!(report.subjects_projected, 1, "only the late one was left");
    assert_eq!(report.subjects_failed, 0);
    assert_eq!(
        frontier_version(&h).await,
        Some(6),
        "one pass carried the frontier through V5 to the already-complete V6"
    );
}

#[tokio::test]
async fn a_straggler_into_a_version_the_frontier_has_passed_is_refused_loudly() {
    // The enforcement that replaces a premise. Completeness is judged from a
    // version's committed refs plus the ones the pass is finalizing, so it is
    // only as good as batch atomicity - one batch, one version, one event. When
    // that fails, a ref of V5 arrives after V5 is already pinnable, and adding
    // a subject to a pinnable version is one pin resolving two contents over
    // time. It is refused at the moment it would corrupt rather than predicted.
    //
    // It also pins the reading of D-163 clause (2) that decides which faults get
    // charged clause (3)'s price. `Ok(None)` is a SUCCESSFUL answer - "not
    // committed yet" - and under clause (1) a ref the registry has not committed
    // cannot belong to a version that already has one, so it does NOT stop the
    // pass judging V5 complete. What the registry then does below is contradict
    // itself, and that is the contract violation this refusal is for. A per-ref
    // registry ERROR is the other thing and defers instead:
    // `a_pass_with_an_unresolvable_sibling_ref_decides_no_completion`.
    let h = harness().await;
    let (_, pending_a) = seed_and_publish_at(&h, "gold", at_min(12, 0)).await;
    let (plan_b, pending_b) = seed_and_publish_at(&h, "silver", at_min(12, 1)).await;
    h.registry.commit(&pending_a, 5);
    h.registry.script(&pending_b, vec![None]);

    let first = sweep(&h, at(13)).await;
    assert_eq!(first.frontiers_advanced, 1);
    assert_eq!(frontier_version(&h).await, Some(5));

    // Now B turns out to have belonged to V5 all along.
    h.registry.commit(&pending_b, 5);
    let second = sweep(&h, at(14)).await;

    assert_eq!(
        second.subjects_failed, 1,
        "the straggler is refused, not quietly projected"
    );
    assert_eq!(second.subjects_projected, 0);

    let rows = deltas(&h).await;
    assert_eq!(rows.len(), 1, "no second subject joined a pinnable version");
    assert!(
        !rows.iter().any(|row| row.subject_ref == plan_b.to_string()),
        "and it is not B's"
    );
    assert_eq!(
        refs(&h)
            .await
            .iter()
            .filter(|row| row.pending_ref == pending_b)
            .filter(|row| row.catalog_version.is_none())
            .count(),
        1,
        "B's ref stays pending, so its age alarms rather than its content landing"
    );
    assert_eq!(frontier_version(&h).await, Some(5), "and nothing moved");
}

// ---------------------------------------------------------------------------
// 4b. The completeness bound: a pass that could not have seen the whole subject
//     set decides no completion (D-163 clause 2).
// ---------------------------------------------------------------------------

/// A version with two subjects of one tenant, both projectable.
///
/// Returns `(the two handles, the version they commit into)`.
async fn seed_two_subject_version(h: &Harness, version: u64) -> (String, String) {
    let (_, first) = seed_and_publish_at(h, "gold", at_min(12, 0)).await;
    let (_, second) = seed_and_publish_at(h, "silver", at_min(12, 1)).await;
    h.registry.commit(&first, version);
    h.registry.commit(&second, version);
    (first, second)
}

#[tokio::test]
async fn a_version_whose_subjects_all_warm_in_one_pass_is_complete_and_advances() {
    // The unchanged base case, asserted on the new report field so the bound is
    // visibly not blocking the ordinary path. A pass that saw both refs and got
    // an answer for both is entitled to count them.
    let h = harness().await;
    let _ = seed_two_subject_version(&h, 5).await;

    let report = sweep(&h, at(13)).await;

    assert_eq!(report.tenants_seen, 1);
    assert_eq!(report.pending_seen, 2);
    assert_eq!(report.subjects_projected, 2);
    assert_eq!(report.versions_complete, 1, "the pass saw the whole set");
    assert_eq!(report.frontiers_advanced, 1);
    assert_eq!(frontier_version(&h).await, Some(5));
}

#[tokio::test]
async fn a_pass_with_an_unresolvable_sibling_ref_decides_no_completion() {
    // D-163 clause (2), the per-ref-outage arm. Before the bound the version read
    // complete from the warm set alone - the unresolvable ref is not in the
    // version's subject set at all, because a pending ref carries no version - the
    // frontier advanced, and the sibling then arrived BELOW it and was refused
    // loudly, at clause (3)'s price: unresolvable at any version, nothing
    // self-healing.
    //
    // Delete `coverage.may_decide_completion()` from either place in
    // `read_model.rs` and this test fails.
    let h = harness().await;
    let (first, second) = seed_two_subject_version(&h, 5).await;
    h.registry.fail_handle(&second);

    let report = sweep(&h, at(13)).await;

    assert_eq!(report.subjects_projected, 1, "the answerable one warmed");
    assert_eq!(
        report.versions_complete, 0,
        "and the pass declined to judge a set it could not have seen whole"
    );
    assert_eq!(report.frontiers_advanced, 0);
    assert_eq!(
        frontier_version(&h).await,
        None,
        "the frontier lags, which is the safe direction"
    );
    // The warmed subject really is warm: the bound gates the COMPLETENESS
    // decision and nothing else about the pass.
    assert_eq!(deltas(&h).await.len(), 1);
    assert!(
        refs(&h)
            .await
            .iter()
            .any(|row| row.pending_ref == first && row.catalog_version == Some(5)),
        "the resolvable ref finalized as usual"
    );
}

#[tokio::test]
async fn the_next_pass_with_the_outage_cleared_completes_the_version_and_advances() {
    // The lag recovers on its own, which is what makes deferring safe rather than
    // merely cautious. Nothing out of band is needed - the sweep arriving again
    // IS the recovery, exactly as it is for the warm re-drive.
    let h = harness().await;
    let (_, second) = seed_two_subject_version(&h, 5).await;
    h.registry.fail_handle(&second);
    sweep(&h, at(13)).await;
    assert_eq!(frontier_version(&h).await, None);

    h.registry.clear_handle_failures();
    let report = sweep(&h, at(14)).await;

    assert_eq!(
        report.subjects_projected, 1,
        "only the deferred one was left"
    );
    assert_eq!(report.versions_complete, 1);
    assert_eq!(report.frontiers_advanced, 1);
    assert_eq!(frontier_version(&h).await, Some(5));
    assert_eq!(deltas(&h).await.len(), 2);
}

#[tokio::test]
async fn a_deferred_completion_is_not_silent() {
    // D-166 clause (5) is what makes clause (2)'s lag reportable rather than
    // invisible: a tenant with no frontier and a ref of its own past the batching
    // SLO is short of pin-eligibility, and that is precisely the deferred state.
    let h = harness().await;
    let (_, second) = seed_two_subject_version(&h, 5).await;
    h.registry.fail_handle(&second);

    let report = sweep(&h, at(13)).await;

    assert_eq!(report.versions_complete, 0);
    assert_eq!(
        report.pin_eligibility_overdue, 1,
        "the deferral is reported, not merely survived"
    );
}

#[tokio::test]
async fn a_saturated_scan_defers_its_own_tenant_and_no_one_elses() {
    // The fairness half, and the one a cross-tenant bound cannot pass. Under the
    // old cross-tenant page ordered by request instant, the tenant holding the
    // oldest refs filled the budget and every OTHER tenant's completions were
    // deferred with it - their frontiers standing still because someone else's
    // backlog was stuck.
    //
    // `pending_refs_per_tenant = 2` and the first tenant holds exactly two, so
    // its page fills; the second holds one and does not. The second tenant's id
    // sorts ABOVE the first's, so it is swept second - the direction that would
    // fail under a shared budget.
    let h = harness_with(JobsConfig {
        pending_refs_per_tenant: 2,
        ..JobsConfig::default()
    })
    .await;
    let _ = seed_two_subject_version(&h, 5).await;
    let (_, other) = seed_and_publish_for(&h, OTHER_TENANT, "gold", at_min(12, 2)).await;
    h.registry.commit(&other, 6);

    let report = sweep(&h, at(13)).await;

    assert_eq!(report.tenants_seen, 2, "both tenants were swept");
    assert_eq!(
        report.subjects_projected, 3,
        "and all three subjects warmed"
    );
    assert_eq!(
        report.versions_complete, 1,
        "one completion: the saturated tenant's version was not judged"
    );
    assert_eq!(
        frontier_version(&h).await,
        None,
        "the saturated tenant's frontier waits for a pass that could see its whole set"
    );
    assert_eq!(
        frontier_version_of(&h, OTHER_TENANT).await,
        Some(6),
        "and the other tenant's advanced in the same pass"
    );
}

#[tokio::test]
async fn a_straggler_below_the_frontier_is_still_refused_once_the_bound_is_in_place() {
    // The bound narrowed `refuse_projection_below_frontier`'s reachability
    // without disarming it. What can no longer reach it is an ordinary per-ref
    // outage; what still does is the registry contradicting D-163 clause (1) -
    // a batch that did not commit atomically into one version.
    //
    // Constructed directly: the frontier is advanced by a complete V5, and then a
    // ref of the SAME tenant is offered at V5 afterwards. No outage is involved,
    // which is the point.
    let h = harness().await;
    let (_, first) = seed_and_publish_at(&h, "gold", at_min(12, 0)).await;
    h.registry.commit(&first, 5);
    sweep(&h, at(13)).await;
    assert_eq!(frontier_version(&h).await, Some(5));

    // A second publish of the tenant that the registry now says belongs to V5 -
    // a version whose subject set clause (1) says closed at the first commit.
    let (plan_b, second) = seed_and_publish_at(&h, "silver", at_min(13, 30)).await;
    h.registry.commit(&second, 5);

    let report = sweep(&h, at(14)).await;

    assert_eq!(
        report.subjects_failed, 1,
        "the straggler is refused, not quietly projected"
    );
    assert_eq!(report.subjects_projected, 0);
    assert_eq!(
        deltas(&h).await.len(),
        1,
        "no subject joined a pinnable version"
    );
    assert!(
        !deltas(&h)
            .await
            .iter()
            .any(|row| row.subject_ref == plan_b.to_string())
    );
    assert_eq!(frontier_version(&h).await, Some(5), "and nothing moved");
}

// ---------------------------------------------------------------------------
// 5. The re-drive is idempotent.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_second_pass_over_a_complete_version_writes_nothing_and_refuses_nothing() {
    // The projector's subtraction IS the re-drive, so a version with nothing
    // outstanding is a pass that does nothing: no second delta row (the primary
    // key would refuse one), no re-finalize, and no attempt to re-advance - a
    // FrontierRegression escaping as a failed pass would be the ordering bug
    // that refusal exists to surface, raised by a mechanism that is working.
    let h = harness().await;
    let (_, pending) = seed_and_publish_at(&h, "gold", at_min(12, 0)).await;
    h.registry.commit(&pending, 1);

    sweep(&h, at(13)).await;
    let after_first = deltas(&h).await;
    let committed_at = refs(&h).await[0].committed_at;

    let report = sweep(&h, at(14)).await;
    assert_eq!(report.subjects_projected, 0, "nothing was outstanding");
    assert_eq!(report.frontiers_advanced, 0);

    assert_eq!(deltas(&h).await, after_first, "not one byte moved");
    assert_eq!(
        refs(&h).await[0].committed_at,
        committed_at,
        "the finalize was not repeated"
    );
    assert_eq!(frontier_version(&h).await, Some(1));
}

// ---------------------------------------------------------------------------
// 6. A registry with no answer leaves the seam exactly as the commit left it.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_registry_that_has_not_committed_the_batch_leaves_the_seam_untouched() {
    // The seam, one tick later. `None` is the registry batching, which D-47
    // budgets at up to five minutes - not an error and not an alarm at this
    // age.
    let h = harness().await;
    let (_, pending) = seed_and_publish_at(&h, "gold", at_min(12, 0)).await;
    h.registry.script(&pending, vec![None]);

    // One minute after the request, which is inside the ratified five-minute
    // max batching delay: the wait is budgeted, so nothing alarms.
    let report = sweep(&h, at(12) + chrono::Duration::minutes(1)).await;
    assert_eq!(report.pending_seen, 1);
    assert_eq!(report.versions_projected, 0);
    assert_eq!(report.commit_overdue, 0);
    assert_eq!(
        report.degraded_emitted, 0,
        "a publish inside the batching SLO is not degraded"
    );
    assert_eq!(
        report.pin_eligibility_overdue, 0,
        "nor is it short of pin-eligibility: the wait is budgeted, and a tenant that has          simply not published for a while is not a stuck one"
    );

    assert!(deltas(&h).await.is_empty());
    assert_eq!(frontier_version(&h).await, None);
    assert_eq!(
        refs(&h).await[0].catalog_version,
        None,
        "the ref is exactly as the commit wrote it"
    );
}

// ---------------------------------------------------------------------------
// 7. A registry that contradicts itself.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_registry_answering_one_handle_two_versions_is_refused() {
    // Re-pointing a committed handle re-points a pin that posted periods
    // resolve through, so the finalize refuses it as an invariant breach rather
    // than accepting the newer answer - and answers the SAME version with `Ok`,
    // because that is the idempotent replay a re-drive is entitled to make.
    //
    // Asked of the repository directly, because through the sweep the case is
    // unreachable: a committed ref is not in `list_pending`, so the sweep never
    // offers the registry a second chance to contradict itself. That is a
    // property worth stating rather than a gap - it is why the guard is a
    // predicate on a statement and not a check in the pass.
    let h = harness().await;
    let (_, pending) = seed_and_publish_at(&h, "gold", at_min(12, 0)).await;
    h.registry.commit(&pending, 5);

    let first = sweep(&h, at(13)).await;
    assert_eq!(first.subjects_projected, 1);
    assert_eq!(frontier_version(&h).await, Some(5));

    let conn = h.provider.conn().expect("conn");
    catalog_version_ref_repo::finalize(
        &conn,
        &h.scope,
        TENANT,
        &pending,
        CatalogVersion::new(5),
        at(14),
    )
    .await
    .expect("the same version again is the idempotent replay");

    let refusal = catalog_version_ref_repo::finalize(
        &conn,
        &h.scope,
        TENANT,
        &pending,
        CatalogVersion::new(6),
        at(14),
    )
    .await
    .expect_err("one handle, two versions, is an invariant breach");
    assert!(
        matches!(refusal, RepoError::CorruptRow(_)),
        "no new variant and no wire code - this path has no client: {refusal:?}"
    );

    let rows = deltas(&h).await;
    assert_eq!(rows.len(), 1, "the subject was projected once, at V5");
    assert_eq!(rows[0].catalog_version, 5);
    assert_eq!(
        refs(&h).await[0].catalog_version,
        Some(5),
        "the refusal moved nothing"
    );
    assert_eq!(frontier_version(&h).await, Some(5));
}

// ---------------------------------------------------------------------------
// 8 + 9. The projection source.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn retirement_carries_its_own_version_rather_than_leaking_into_an_older_one() {
    // D-128 against a database, and it is a statement about **two** versions.
    // Retirement is a publish unit of its own precisely because a retired plan
    // can never publish again, so nothing later would re-project it - and being
    // a unit, it requests its own `CatalogVersion` and pins its own state.
    //
    // So V5, published before the retirement, keeps saying `published`: at V5
    // the plan really was sellable, and a delta that said otherwise would be a
    // frozen version changing its mind. V6, the retirement's own version, says
    // `retired`, which is what sellability predicate (4) reads at the pin.
    let h = harness().await;
    let (plan_id, pending_v5) = seed_and_publish_at(&h, "gold", at_min(12, 0)).await;

    // The retirement unit: the flip, and the ref it would request. Written by
    // statement because retirement has no writer in this gear at all - the
    // group that lands it is not this one - and the flip is one the
    // `pricing_plan` trigger whitelist explicitly sanctions.
    retire(&h, plan_id).await;
    let conn = h.provider.conn().expect("conn");
    catalog_version_ref_repo::record_pending(
        &conn,
        &h.scope,
        PendingVersionRow::for_subject(
            TENANT,
            "pend-retire".to_owned(),
            &SubjectRef::Plan(plan_id.get()),
            Some(0),
            Some(LifecycleState::Retired),
            at_min(12, 1),
        ),
    )
    .await
    .expect("record the retirement unit's ref");

    h.registry.commit(&pending_v5, 5);
    h.registry.commit("pend-retire", 6);
    sweep(&h, at(13)).await;

    let rows = deltas(&h).await;
    assert_eq!(rows.len(), 2, "one delta per version, same subject");
    let v5 = rows
        .iter()
        .find(|row| row.catalog_version == 5)
        .expect("V5's delta");
    let v6 = rows
        .iter()
        .find(|row| row.catalog_version == 6)
        .expect("V6's delta");

    assert_eq!(
        v5.payload.get("lifecycleState"),
        Some(&serde_json::json!("published")),
        "the retirement did not leak backwards into a version that froze before it"
    );
    assert_eq!(
        v6.payload.get("lifecycleState"),
        Some(&serde_json::json!("retired")),
        "and it arrives carrying its own version"
    );
    assert_eq!(frontier_version(&h).await, Some(6));
}

#[tokio::test]
async fn a_delta_never_freezes_a_superseded_lifecycle_state() {
    // The residue pinning the revision alone left, closed. Read live, the
    // pinned revision's state is `superseded` the moment its successor commits
    // - a third value D-128 does not contemplate for a projected subject, and
    // one `load_current` could never return. Frozen into an INSERT-only delta
    // on the seven-year horizon it makes the version read unsellable to a
    // consumer coding sellability predicate (4) as "is published", which is how
    // D-90 names that predicate's input.
    let h = harness().await;
    let plan_id = PlanId::new(Uuid::new_v4());
    let (rev0, version0) = seed_publishable(&h, plan_id, "gold").await;
    let pending_v5 = publish(&h, plan_id, rev0, version0, at_min(12, 0)).await;

    // The successor publishes before V5 warms, so revision 0 is `superseded` in
    // the truth table by the time the sweep arrives.
    let opened = h
        .plans
        .open_revision(&h.scope, TENANT, plan_id, ACTOR, at_min(12, 1))
        .await
        .expect("open a successor");
    let pending_v6 = publish(
        &h,
        plan_id,
        opened.revision,
        opened.row_version,
        at_min(12, 2),
    )
    .await;
    assert_eq!(
        h.plans
            .find_revision(&h.scope, TENANT, plan_id, rev0)
            .await
            .expect("read revision 0")
            .expect("it is there")
            .lifecycle_state,
        LifecycleState::Superseded,
        "the truth row really is superseded, which is what makes this a real case"
    );

    h.registry.commit(&pending_v5, 5);
    h.registry.commit(&pending_v6, 6);
    sweep(&h, at(13)).await;

    let rows = deltas(&h).await;
    for row in &rows {
        assert_ne!(
            row.payload.get("lifecycleState"),
            Some(&serde_json::json!("superseded")),
            "no delta may carry a state D-128 does not contemplate: {row:?}"
        );
    }
    assert_eq!(
        rows.iter()
            .find(|row| row.catalog_version == 5)
            .expect("V5's delta")
            .payload
            .get("lifecycleState"),
        Some(&serde_json::json!("published")),
        "V5 froze the state its own publish judged"
    );
}

#[tokio::test]
async fn the_open_draft_revision_is_never_the_projection_source() {
    // Sec 4.4's 2026-07-30 review fix, as behaviour: a degraded re-drive must
    // not leak draft edits into a frozen version. It is a property of calling
    // load_current and never load_open_draft, so the assertion is that a draft
    // carrying different content changes nothing about the delta.
    let h = harness().await;
    let (plan_id, pending) = seed_and_publish_at(&h, "gold", at_min(12, 0)).await;

    // A successor revision, open and edited, with a tier the published one
    // does not have.
    let opened = h
        .plans
        .open_revision(&h.scope, TENANT, plan_id, ACTOR, at(12))
        .await
        .expect("open a successor draft");
    h.plans
        .update_draft(
            &h.scope,
            TENANT,
            plan_id,
            opened.revision,
            opened.row_version,
            PlanShapePatch {
                plan_tier: Some("platinum".to_owned()),
                ..PlanShapePatch::default()
            },
            stamp(),
        )
        .await
        .expect("edit the draft");

    h.registry.commit(&pending, 1);
    sweep(&h, at(13)).await;

    let rows = deltas(&h).await;
    assert_eq!(
        rows[0].payload.get("planTier"),
        Some(&serde_json::json!("gold")),
        "the current revision's content, never the open draft's"
    );
    assert_eq!(
        rows[0].payload.get("revision"),
        Some(&serde_json::json!(0)),
        "and the current revision's number with it"
    );
}

#[tokio::test]
async fn a_version_freezes_the_revision_its_own_publish_judged() {
    // The reviewer's reproduction, and the reason the ref row carries the
    // revision. sec 4.4 sources the projection from the plan's CURRENT revision,
    // and the sweep arrives up to the D-47 maximum batching delay - five
    // minutes - after the commit. A second publish of the same plan inside that
    // window makes its own revision current, so "current" freezes content the
    // earlier version's publish never judged. Permanently: a delta is
    // INSERT-only on the seven-year horizon, in a store whose whole contract is
    // that a completed version never changes.
    let h = harness().await;
    let plan_id = PlanId::new(Uuid::new_v4());
    let (rev0, version0) = seed_publishable(&h, plan_id, "gold").await;
    let pending_v5 = publish(&h, plan_id, rev0, version0, at_min(12, 0)).await;

    // A successor revision with different content, published before the first
    // version's warm.
    let opened = h
        .plans
        .open_revision(&h.scope, TENANT, plan_id, ACTOR, at_min(12, 1))
        .await
        .expect("open a successor");
    let edited = h
        .plans
        .update_draft(
            &h.scope,
            TENANT,
            plan_id,
            opened.revision,
            opened.row_version,
            PlanShapePatch {
                plan_tier: Some("platinum".to_owned()),
                ..PlanShapePatch::default()
            },
            stamp(),
        )
        .await
        .expect("edit the successor");
    let pending_v6 = publish(
        &h,
        plan_id,
        opened.revision,
        edited.row_version,
        at_min(12, 2),
    )
    .await;

    // Both batches commit, and one sweep resolves both.
    h.registry.commit(&pending_v5, 5);
    h.registry.commit(&pending_v6, 6);
    sweep(&h, at(13)).await;

    let rows = deltas(&h).await;
    assert_eq!(rows.len(), 2, "one delta per version");
    let v5 = rows
        .iter()
        .find(|row| row.catalog_version == 5)
        .expect("V5's delta");
    let v6 = rows
        .iter()
        .find(|row| row.catalog_version == 6)
        .expect("V6's delta");

    assert_eq!(
        v5.payload.get("revision"),
        Some(&serde_json::json!(rev0)),
        "V5 froze the revision its own publish judged"
    );
    assert_eq!(
        v5.payload.get("planTier"),
        Some(&serde_json::json!("gold")),
        "and that revision's content, not the one that overtook it"
    );
    assert_eq!(
        v6.payload.get("revision"),
        Some(&serde_json::json!(opened.revision))
    );
    assert_eq!(
        v6.payload.get("planTier"),
        Some(&serde_json::json!("platinum"))
    );
}

// ---------------------------------------------------------------------------
// 10. The degraded observation, and the instant that separates it from the
//     overdue one (D-166).
// ---------------------------------------------------------------------------

/// A ref whose commit the registry answers and whose subject can never project,
/// so the pass observes the commit and the warm keeps failing — the one state
/// `fr-publish-fanout-atomicity` names and the merged predicate could not.
///
/// Returns the handle.
async fn seed_observed_but_unwarm(
    h: &Harness,
    version: u64,
    requested_at: DateTime<Utc>,
) -> String {
    let conn = h.provider.conn().expect("conn");
    catalog_version_ref_repo::record_pending(
        &conn,
        &h.scope,
        PendingVersionRow::for_subject(
            TENANT,
            "pend-stuck".to_owned(),
            &SubjectRef::Plan(Uuid::new_v4()),
            Some(0),
            Some(LifecycleState::Published),
            requested_at,
        ),
    )
    .await
    .expect("record the stuck subject");
    h.registry.commit("pend-stuck", version);
    "pend-stuck".to_owned()
}

/// A ref's recorded commit observation.
async fn observed_at(h: &Harness, pending_ref: &str) -> Option<DateTime<Utc>> {
    refs(h)
        .await
        .into_iter()
        .find(|row| row.pending_ref == pending_ref)
        .expect("the ref is there")
        .commit_observed_at
}

#[tokio::test]
async fn a_pending_ref_the_registry_has_not_answered_is_overdue_and_not_degraded() {
    // D-166 clause (4). Before the observation instant existed BOTH signals
    // fired here, off one clock, so an operator could not tell "the registry has
    // not answered" from "the registry answered and the warm is failing" - the
    // one distinction `fr-publish-fanout-atomicity` exists to draw.
    let h = harness().await;
    let (_, pending) = seed_and_publish_at(&h, "gold", at_min(12, 0)).await;
    h.registry.script(&pending, vec![None]);

    // Requested at 12:00; the ratified max batching delay is five minutes, so by
    // 13:00 the registry is overdue - and by 14:00 it still is.
    let first = sweep(&h, at(13)).await;
    assert_eq!(first.commit_overdue, 1);
    assert_eq!(
        first.degraded_emitted, 0,
        "there is nothing degraded about a publish whose version does not exist yet"
    );
    let second = sweep(&h, at(14)).await;
    assert_eq!(second.commit_overdue, 1, "still unanswered, still alarming");
    assert_eq!(second.degraded_emitted, 0);

    assert!(
        degraded_events(&h).await.is_empty(),
        "and no event was enqueued for it"
    );
    assert_eq!(
        observed_at(&h, &pending).await,
        None,
        "nothing was observed, so nothing is stamped"
    );
}

#[tokio::test]
async fn a_registry_that_errors_still_trips_the_commit_overdue_alarm() {
    // sec 3.6 conditions `commit_overdue` on the ref's age and on whether the
    // commit has been observed - never on what the registry said THIS pass. An
    // earlier version raised it only when the registry answered "not yet", so a
    // registry that ERRORED left the ref ageing in silence however long it stood,
    // which is the outage the alarm most needs to name.
    let h = harness().await;
    let (_, pending) = seed_and_publish_at(&h, "gold", at_min(12, 0)).await;
    // No answer is scripted, and the double answers `Ok(None)` for an unknown
    // handle; script an outage instead.
    h.registry
        .fail_with(&CatalogVersionRegistryError::Unreachable("down".to_owned()));

    let report = sweep(&h, at(13)).await;

    assert_eq!(report.versions_projected, 0);
    assert_eq!(
        report.commit_overdue, 1,
        "the ref aged past the SLO with no observation, whatever the registry said"
    );
    assert_eq!(
        report.degraded_emitted, 0,
        "an unreachable registry is not a warm that is failing"
    );
    assert_eq!(observed_at(&h, &pending).await, None);
}

#[tokio::test]
async fn an_observed_commit_is_stamped_even_though_its_projection_fails() {
    // The placement is the mechanism. The projector finalizes a ref and writes
    // its delta warm in ONE transaction, so an observation stamped inside it
    // would roll back on exactly the path the degraded signal exists for.
    //
    // Delete `observe_commit`'s call in `resolve` and this test fails, along
    // with every degraded assertion below.
    let h = harness().await;
    let pending = seed_observed_but_unwarm(&h, 5, at_min(12, 1)).await;

    let report = sweep(&h, at(13)).await;

    assert_eq!(report.subjects_failed, 1, "the subject cannot project");
    assert_eq!(
        observed_at(&h, &pending).await,
        Some(at(13)),
        "and the observation is recorded anyway"
    );
    assert_eq!(
        refs(&h)
            .await
            .into_iter()
            .find(|row| row.pending_ref == pending)
            .expect("the ref is there")
            .catalog_version,
        None,
        "while the ref itself is still unresolved - the state that is unreachable \
         without the column"
    );
}

#[tokio::test]
async fn the_observation_instant_is_the_first_sighting_and_does_not_move() {
    // Write-once, and it is the whole of whether the signal ever raises: a stamp
    // that advanced every pass would hold the degraded age at zero forever.
    let h = harness().await;
    let pending = seed_observed_but_unwarm(&h, 5, at_min(12, 1)).await;

    sweep(&h, at_min(13, 0)).await;
    sweep(&h, at_min(13, 30)).await;

    assert_eq!(
        observed_at(&h, &pending).await,
        Some(at_min(13, 0)),
        "the second pass re-observed the same commit and left the instant alone"
    );
}

#[tokio::test]
async fn an_observed_commit_is_degraded_only_once_the_propagation_slo_has_passed() {
    // D-166 clause (2), both directions. sec 1.2's SLO is 5s, and the pass that
    // first observes the commit observes it at its own `now` - so the pass that
    // stamps can never be the pass that alarms, which is the correct shape: the
    // warm was attempted in that same pass.
    let h = harness().await;
    let pending = seed_observed_but_unwarm(&h, 5, at_min(12, 1)).await;

    let stamping = sweep(&h, at(13)).await;
    assert_eq!(
        stamping.degraded_emitted, 0,
        "an observation zero seconds old is inside the SLO"
    );

    let inside = sweep(&h, at(13) + chrono::Duration::seconds(4)).await;
    assert_eq!(
        inside.degraded_emitted, 0,
        "four seconds after the observation is still inside it"
    );

    let outside = sweep(&h, at(13) + chrono::Duration::seconds(5)).await;
    assert_eq!(
        outside.degraded_emitted, 1,
        "five seconds is the SLO, and the warm has not landed"
    );
    assert_eq!(
        outside.commit_overdue, 0,
        "and it is NOT overdue - the registry answered"
    );

    let events = degraded_events(&h).await;
    assert_eq!(events.len(), 1, "one degradation, one event");
    assert_eq!(
        events[0].payload.get("catalogVersion"),
        Some(&serde_json::json!(5)),
        "the event names the publish by its version, which D-166 makes knowable"
    );
    assert_eq!(
        events[0].payload.get("commitObservedAt"),
        Some(&serde_json::json!(at(13))),
        "and measures the wait from the observation, not from the request"
    );
    assert_eq!(
        events[0].payload.get("pendingVersionRef"),
        Some(&serde_json::json!(pending)),
        "the handle rides as lineage"
    );
    assert_eq!(
        events[0].published_at, None,
        "the relay drains, not the sweep"
    );

    // The dedup index makes a repeat of one degradation one event.
    let again = sweep(&h, at(14)).await;
    assert_eq!(
        again.degraded_emitted, 0,
        "the dedup index refused the repeat"
    );
    assert_eq!(degraded_events(&h).await.len(), 1);
}

#[tokio::test]
async fn an_observed_commit_is_never_overdue_however_long_it_stays_unresolved() {
    // D-166 clause (4)'s other side, and the reason the two signals are now
    // disjoint over one clock rather than merely differently thresholded: the
    // ref below is HOURS past the max batching delay and the registry answered
    // it, so `commit_overdue` must stay silent whatever its age.
    let h = harness().await;
    let _ = seed_observed_but_unwarm(&h, 5, at_min(12, 1)).await;

    sweep(&h, at(13)).await;
    let much_later = sweep(&h, at(23)).await;
    let later_still = sweep(&h, at(23) + chrono::Duration::hours(5)).await;

    assert_eq!(
        much_later.commit_overdue, 0,
        "the registry answered; ageing past its budget says nothing about it now"
    );
    assert_eq!(
        later_still.commit_overdue, 0,
        "and it stays silent however long the warm keeps failing"
    );
    assert_eq!(
        degraded_events(&h).await.len(),
        1,
        "the degradation is reported once and by the other signal"
    );
}

#[tokio::test]
async fn every_pending_state_lands_on_exactly_one_signal_or_none() {
    // The table D-166 clause (4) closes with: "nothing goes silent". Four
    // combinations of (observed?, age past its own threshold?), each asserted to
    // produce exactly one of the two signals or neither - which is what a merged
    // predicate could not express, both of its arms being true at once.
    //
    // The tenant has no frontier, so `pin_eligibility_overdue` is a THIRD signal
    // riding along on the observed cases; it is asserted where it belongs
    // (`a_stale_frontier_...`) and read here only to show it is not what carries
    // these two.
    for (unanswered, earlier_pass, now, overdue, degraded, label) in [
        (
            true,
            None,
            at_min(12, 1),
            0,
            0,
            "unanswered and inside the batching SLO: neither",
        ),
        (
            true,
            None,
            at(13),
            1,
            0,
            "unanswered and past the batching SLO: overdue only",
        ),
        (
            false,
            None,
            at_min(12, 1),
            0,
            0,
            "answered, warm attempted this pass, observation zero seconds old: neither",
        ),
        (
            false,
            Some(at_min(12, 1)),
            at(13),
            0,
            1,
            "answered and unwarm past the propagation SLO: degraded only",
        ),
    ] {
        let h = harness().await;
        let pending = seed_observed_but_unwarm(&h, 5, at_min(12, 0)).await;
        if unanswered {
            h.registry.script(&pending, vec![None]);
        }
        if let Some(stamping) = earlier_pass {
            // Stamp the observation on an earlier pass, so the age measured at
            // `now` is the observation's and not zero.
            sweep(&h, stamping).await;
        }

        let report = sweep(&h, now).await;

        assert_eq!(report.commit_overdue, overdue, "{label}");
        assert_eq!(report.degraded_emitted, degraded, "{label}");
        assert!(
            report.commit_overdue == 0 || report.degraded_emitted == 0,
            "the two signals may never both fire on one ref: {label}"
        );
    }
}

#[tokio::test]
async fn a_committed_version_whose_projection_keeps_failing_holds_the_frontier_and_says_so() {
    // sec 4.4's own words: "a stuck version now holds the frontier, which is
    // exactly what that alarm signals". Under D-166 this path is DEGRADED rather
    // than overdue - the registry answered - and `pin_eligibility_overdue` is
    // what says the frontier is held.
    let h = harness().await;
    let (_, pending_a) = seed_and_publish_at(&h, "gold", at_min(12, 0)).await;
    let _ = seed_observed_but_unwarm(&h, 5, at_min(12, 1)).await;
    h.registry.commit(&pending_a, 5);

    sweep(&h, at(13)).await;
    let report = sweep(&h, at(14)).await;

    assert_eq!(report.subjects_failed, 1, "the stuck subject refuses");
    assert_eq!(
        frontier_version(&h).await,
        None,
        "so V5 never becomes pinnable"
    );
    assert_eq!(
        report.commit_overdue, 0,
        "the registry answered, so this is not its fault"
    );
    assert_eq!(
        report.degraded_emitted, 1,
        "the version committed and the warm has not landed"
    );
    assert_eq!(
        report.pin_eligibility_overdue, 1,
        "and a committed version stands short of pin-eligibility with no frontier at all"
    );
}

#[tokio::test]
async fn an_ordinary_publish_behind_an_old_frontier_does_not_alarm() {
    // The second half of the same regression. A tenant that published an hour
    // ago has a stale frontier by any measure, and a fresh publish of it is
    // simply waiting out D-47's budget - so staleness conjoined with "has a
    // pending ref" would raise Critical on a healthy catalog every tick. The
    // ref has to be overdue itself.
    let h = harness().await;
    let (_, first) = seed_and_publish_at(&h, "gold", at_min(12, 0)).await;
    h.registry.commit(&first, 5);
    sweep(&h, at_min(12, 1)).await;
    assert_eq!(frontier_version(&h).await, Some(5));

    // An hour later, a new publish that the registry has not committed yet.
    let (_, second) = seed_and_publish_at(&h, "silver", at_min(13, 0)).await;
    h.registry.script(&second, vec![None]);

    let report = sweep(&h, at_min(13, 1)).await;

    assert_eq!(
        report.pin_eligibility_overdue, 0,
        "the frontier is an hour old and the publish is one minute old; nothing is stuck"
    );
    assert_eq!(report.commit_overdue, 0);
}

#[tokio::test]
async fn a_stale_frontier_with_a_version_waiting_behind_it_alarms_on_its_own_age() {
    // D-136 and PinFrontier::advanced_at both name the frontier's age as this
    // alarm's referent. The tenant below has a frontier, so nothing about a
    // pending ref's age is what makes it overdue - the frontier itself has not
    // moved within the SLO while a committed version waits.
    let h = harness().await;
    let (_, pending_a) = seed_and_publish_at(&h, "gold", at_min(12, 0)).await;
    h.registry.commit(&pending_a, 5);
    sweep(&h, at_min(12, 1)).await;
    assert_eq!(frontier_version(&h).await, Some(5));

    // A later version whose subject cannot project, so V6 stands committed and
    // short of pin-eligibility while the frontier ages at V5.
    let conn = h.provider.conn().expect("conn");
    catalog_version_ref_repo::record_pending(
        &conn,
        &h.scope,
        PendingVersionRow::for_subject(
            TENANT,
            "pend-stuck".to_owned(),
            &SubjectRef::Plan(Uuid::new_v4()),
            Some(0),
            Some(LifecycleState::Published),
            at_min(12, 2),
        ),
    )
    .await
    .expect("record the stuck subject");
    h.registry.commit("pend-stuck", 6);

    let report = sweep(&h, at(14)).await;

    assert_eq!(
        report.pin_eligibility_overdue, 1,
        "the frontier last advanced at 12:01 and a committed V6 is waiting"
    );
}

// ---------------------------------------------------------------------------
// The inert pass G7's e2e depends on.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_sweep_with_no_registry_configured_is_inert() {
    // G7's e2e boots without a registry. A pass that logged an error every five
    // seconds would make that boot unreadable, so the unconfigured answer
    // returns the pass at debug: no alarm, no error, nothing written.
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    let provider = DBProvider::<DbError>::new(db);
    let registry = Arc::new(RegistryDouble::default());
    let h = Harness {
        plans: PlanRepo::new(provider.clone()),
        shapes: PlanShapeRepo::new(provider.clone()),
        prices: PriceRepo::new(provider.clone()),
        publish: PublishService::new(
            provider.clone(),
            &LimitsConfig::default(),
            FixtureGate::load(&committed_registry_path()),
            Arc::clone(&registry) as Arc<dyn CatalogVersionRegistryV1>,
        ),
        frontier: PinFrontierRepo::new(provider.clone()),
        job: ReadModelWarmJob::new(
            provider.clone(),
            Arc::new(
                bss_pricing_sdk::catalog_version_registry::UnconfiguredCatalogVersionRegistryV1,
            ),
            JobsConfig::default(),
        ),
        registry,
        scope: AccessScope::for_tenant(TENANT),
        provider,
    };
    let (_, _) = seed_and_publish_at(&h, "gold", at_min(12, 0)).await;

    let report = sweep(&h, at(13)).await;

    assert!(report.inert, "no registry means no pass");
    assert_eq!(report.versions_projected, 0);
    assert_eq!(
        report.commit_overdue, 0,
        "an absent registry is not an alarm"
    );
    assert!(deltas(&h).await.is_empty());
    assert_eq!(frontier_version(&h).await, None);
}

/// The actor and instant every mutating repository call now records (D-135 - the
/// audit row commits inside the mutation's own transaction).
fn stamp() -> bss_pricing::domain::audit::AuditStamp {
    bss_pricing::domain::audit::AuditStamp {
        actor_principal_id: uuid::Uuid::from_u128(0xac_10),
        recorded_at: chrono::Utc::now(),
        correlation_id: None,
    }
}
