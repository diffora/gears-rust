//! The publish commit as a whole, against a real database.
//!
//! `design/01-foundation.md` §4.2 step 4 is five moves in one transaction, and
//! what this suite proves is the "one transaction" half: after a success the
//! database holds exactly five artifacts, and after **any** failure it holds
//! none of them. A unit test over the service could assert the calls happen; it
//! could not assert that a failure three statements in undoes the two before it,
//! which is the only property that makes a publish atomic.
//!
//! # The G5/G6 seam is asserted here, deliberately
//!
//! Every success case asserts `pricing_read_model` is **empty** and
//! `pricing_pin_frontier` is **untouched**. Those two assertions are the seam:
//! the commit holds a *pending* handle and no version number, and
//! `pricing_read_model` is keyed `catalog_version NOT NULL`, so a delta row is
//! not even expressible here. They are the tests that fail if G6's work ever
//! leaks backwards into this one.
//!
//! # The registry double, and why it may never leave this file
//!
//! The only registry this crate can have in production is
//! `UnconfiguredCatalogVersionRegistryV1`, because the registry gear has no code
//! in this repository — so the **fail-closed arm is the only arm the real wiring
//! can reach**, and the success path needs a double. It lives in the test crate
//! and must never move into `src/`, not even as a "development" default: the
//! single failure this port exists to prevent is a second incrementer shipping,
//! and a default that invents versions locally is exactly that. The double also
//! asserts the idempotency the commit depends on — two calls carrying one
//! `request_id` return one handle — because that is what makes a rolled-back and
//! retried commit re-request rather than orphan.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bss_pricing::config::LimitsConfig;
use bss_pricing::domain::audit::{
    AuditAction, AuditRecord, AuditSubjectKind, audit_row_hash, genesis_prev_hash,
};
use bss_pricing::domain::concurrency::RowVersion;
use bss_pricing::domain::error::DomainError;
use bss_pricing::domain::evaluation_policy::EVALUATION_POLICY_GENERATION;
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::money::{CurrencyCode, MinorAmount};
use bss_pricing::domain::plan_shape::{PhaseKind, PlanPhase};
use bss_pricing::domain::price_record::PriceContent;
use bss_pricing::domain::price_row::{ModelKind, PriceRow};
use bss_pricing::domain::publish::{PlanPublishUnit, PublishAuthorization};
use bss_pricing::domain::read_model::SubjectKind;
use bss_pricing::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use bss_pricing::domain::snapshot::VersionRef;
use bss_pricing::infra::fixture_gate::FixtureGate;
use bss_pricing::infra::publish::PublishService;
use bss_pricing::infra::storage::entity::{
    audit_log, catalog_version_ref, outbox, pin_frontier, read_model,
};
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::{
    NewOutboxEvent, NewPlanDraft, NewPriceDraft, PlanPublishedPayload, PlanRepo, PlanShapeRepo,
    PriceRepo, outbox_repo,
};
use bss_pricing_sdk::catalog_version::CatalogVersion;
use bss_pricing_sdk::catalog_version_registry::{
    CatalogVersionRegistryError, CatalogVersionRegistryV1, PendingVersionRef,
    UnconfiguredCatalogVersionRegistryV1,
};
use chrono::{DateTime, TimeZone, Utc};
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use sea_orm_migration::MigratorTrait;
use std::path::{Path, PathBuf};
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::{AccessScope, SecureEntityExt, SecureInsertExt};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use toolkit_security::SecurityContext;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// The registry double.
// ---------------------------------------------------------------------------

/// A registry that hands out one pending handle per `request_id`.
///
/// Idempotent on purpose: the commit derives its `request_id` deterministically
/// from `(tenant, plan, revision)` so that a rolled-back-and-retried publish
/// re-requests the same assignment, and a double that minted a fresh handle per
/// call would let that property pass untested.
#[derive(Default)]
struct RegistryDouble {
    issued: Mutex<HashMap<String, String>>,
}

impl RegistryDouble {
    fn calls(&self) -> usize {
        self.issued.lock().expect("no panics in the double").len()
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
    ) -> Result<Option<CatalogVersion>, CatalogVersionRegistryError> {
        // G6's call, and nothing in this suite makes it.
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// The harness.
// ---------------------------------------------------------------------------

const TENANT: Uuid = Uuid::from_u128(0x7e_11);
const ACTOR: Uuid = Uuid::from_u128(0xac_10);
const CORRELATION: Uuid = Uuid::from_u128(0xc0_11);

fn plan_id() -> PlanId {
    PlanId::new(Uuid::from_u128(0x9_1a4))
}

fn terminal_phase() -> PhaseId {
    PhaseId::new(Uuid::from_u128(0xfa_5e))
}

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 3, hour, 0, 0).unwrap()
}

fn ctx() -> SecurityContext {
    SecurityContext::builder()
        .subject_id(ACTOR)
        .subject_tenant_id(TENANT)
        .build()
        .expect("a subject and a tenant are all a context needs")
}

/// The committed corpus registry, resolved from this crate's manifest so the
/// suite does not depend on the directory `cargo test` was invoked from.
fn committed_registry_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus/registry.toml")
}

struct Harness {
    plans: PlanRepo,
    shapes: PlanShapeRepo,
    prices: PriceRepo,
    publish: PublishService,
    registry: Arc<RegistryDouble>,
    scope: AccessScope,
    provider: DBProvider<DbError>,
}

async fn harness() -> Harness {
    harness_with(Arc::new(RegistryDouble::default())).await
}

async fn harness_with(registry: Arc<RegistryDouble>) -> Harness {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    let provider = DBProvider::<DbError>::new(db);
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
        registry,
        scope: AccessScope::for_tenant(TENANT),
        provider,
    }
}

fn new_plan_draft() -> NewPlanDraft {
    NewPlanDraft {
        plan_id: plan_id(),
        tenant_id: TENANT,
        created_by: ACTOR,
        created_at_utc: at(10),
        sku_id: Some(Uuid::from_u128(0x5_c1)),
        plan_tier: Some("gold".to_owned()),
        billing_cycle: Some(bss_pricing::domain::plan_shape::BillingCycle::Recurring),
        frequency: Some(bss_pricing::domain::plan_shape::Frequency::Monthly),
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
        // Its own policy, so the Foundation's rounding rule resolves without a
        // tenant policy row.
        rounding_policy_ref: Some("half_up".to_owned()),
        grandfather_until: None,
        supersedes_price_id: None,
    }
}

fn scope_key(eligibility: PriceEligibility) -> ScopeKey {
    ScopeKey::new(
        plan_id(),
        CurrencyCode::new("EUR").expect("three letters"),
        Region::new("eu").expect("a non-blank region"),
        terminal_phase(),
        eligibility,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("the class pairs with cohort none")
}

/// A plan the whole rule set passes: one evergreen terminal phase, a descriptor
/// set, a tier, and one flat recurring row.
async fn seed_publishable(h: &Harness) -> (u64, RowVersion, Uuid) {
    let created = h
        .plans
        .create_draft(&h.scope, new_plan_draft())
        .await
        .expect("create the draft");
    let after_phases = h
        .shapes
        .replace_phases(
            &h.scope,
            TENANT,
            plan_id(),
            created.revision,
            created.row_version,
            vec![PlanPhase {
                phase_id: terminal_phase(),
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
            &h.scope,
            TENANT,
            plan_id(),
            created.revision,
            after_phases.row_version,
            bss_pricing::domain::plan_shape::DescriptorSet {
                invoice_line_template: Some("{plan}".to_owned()),
                gl_code: Some("4000".to_owned()),
                itemization_rule: Some("per_charge".to_owned()),
                additional: std::collections::BTreeMap::new(),
            },
            stamp(),
        )
        .await
        .expect("attach the descriptor set");

    let price_id = Uuid::from_u128(0xb_0001);
    h.prices
        .create_draft(
            &h.scope,
            TENANT,
            NewPriceDraft {
                price_id,
                scope_key: scope_key(PriceEligibility::AllSubscriptions),
                content: flat_row(),
                created_by: ACTOR,
                created_at_utc: at(10),
            },
        )
        .await
        .expect("author the price row");

    (created.revision, after_descriptors.row_version, price_id)
}

// ---------------------------------------------------------------------------
// Reading the five artifacts back.
// ---------------------------------------------------------------------------

async fn version_refs(h: &Harness) -> Vec<catalog_version_ref::Model> {
    let conn = h.provider.conn().expect("conn");
    catalog_version_ref::Entity::find()
        .secure()
        .scope_with(&h.scope)
        .all(&conn)
        .await
        .expect("read the version refs")
}

async fn outbox_rows(h: &Harness) -> Vec<outbox::Model> {
    let conn = h.provider.conn().expect("conn");
    outbox::Entity::find()
        .secure()
        .scope_with(&h.scope)
        .filter(Condition::all().add(outbox::Column::TenantId.eq(TENANT)))
        .all(&conn)
        .await
        .expect("read the outbox")
}

async fn audit_rows(h: &Harness) -> Vec<audit_log::Model> {
    let conn = h.provider.conn().expect("conn");
    audit_log::Entity::find()
        .secure()
        .scope_with(&h.scope)
        .filter(Condition::all().add(audit_log::Column::TenantId.eq(TENANT)))
        .all(&conn)
        .await
        .expect("read the audit chain")
}

/// The chain's **publish** records, in seq order.
///
/// The authoring seeds this suite builds a publishable plan with are themselves
/// audited now (D-135, G8) - a create and three facet edits - so a count over the
/// whole segment stopped being a count of what a publish wrote. Filtering by
/// action is the honest narrowing: this suite's subject is the publish commit,
/// and the seeds' records are `sqlite_audit_chain.rs`'s.
async fn publish_records(h: &Harness) -> Vec<audit_log::Model> {
    let mut rows: Vec<audit_log::Model> = audit_rows(h)
        .await
        .into_iter()
        .filter(|row| row.action == "publish")
        .collect();
    rows.sort_by_key(|row| row.seq);
    rows
}

/// How many records the authoring seeds leave on the plan's segment: the plan
/// create, plus one per facet the seed sets (phases, add-on rules, descriptors).
const SEEDED_AUTHORING_RECORDS: i64 = 4;

async fn read_model_rows(h: &Harness) -> Vec<read_model::Model> {
    let conn = h.provider.conn().expect("conn");
    read_model::Entity::find()
        .secure()
        .scope_with(&h.scope)
        .all(&conn)
        .await
        .expect("read the read model")
}

async fn frontier_rows(h: &Harness) -> Vec<pin_frontier::Model> {
    let conn = h.provider.conn().expect("conn");
    pin_frontier::Entity::find()
        .secure()
        .scope_with(&h.scope)
        .all(&conn)
        .await
        .expect("read the pin frontier")
}

/// The G5/G6 seam, asserted rather than assumed.
async fn assert_seam_holds(h: &Harness) {
    assert!(
        read_model_rows(h).await.is_empty(),
        "a commit holds a pending handle and no version, so no delta row is even \
         expressible: pricing_read_model is G6's"
    );
    assert!(
        frontier_rows(h).await.is_empty(),
        "the pin frontier advances only inside the transaction that completes the \
         frontier's next version in order, which is G6's"
    );
}

// ---------------------------------------------------------------------------
// The success path.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_first_publish_leaves_exactly_the_five_artifacts_and_nothing_else() {
    let h = harness().await;
    let (revision, version, price_id) = seed_publishable(&h).await;

    let receipt = h
        .publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            PlanPublishUnit::new(plan_id(), revision),
            version,
            PublishAuthorization::auto_publishable(),
            ACTOR,
            CORRELATION,
            at(12),
        )
        .await
        .expect("the publish commits");

    // The receipt holds a handle, never a version.
    assert_eq!(receipt.version_ref(), &VersionRef::Pending("pend-0".into()));
    assert_eq!(receipt.published_price_ids(), [price_id]);
    // The publish record extends the segment the authoring seeds already built,
    // so its position is after theirs rather than at genesis.
    assert_eq!(
        i64::try_from(receipt.audit_seq()).expect("a small seq"),
        SEEDED_AUTHORING_RECORDS
    );

    // 1. The revision is published and is the plan's current one.
    let current = h
        .plans
        .find_current(&h.scope, TENANT, plan_id())
        .await
        .expect("read the current revision")
        .expect("the plan has one");
    assert_eq!(current.revision, revision);
    assert_eq!(current.lifecycle_state, LifecycleState::Published);

    // 2. The price row is published.
    let row = h
        .prices
        .find(&h.scope, TENANT, price_id)
        .await
        .expect("read the row")
        .expect("it is there");
    assert_eq!(row.lifecycle_state, LifecycleState::Published);

    // 3. One pending ref, uncommitted, naming the subject the projector needs.
    let refs = version_refs(&h).await;
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].pending_ref, "pend-0");
    assert_eq!(refs[0].catalog_version, None);
    assert_eq!(refs[0].committed_at, None);
    assert_eq!(refs[0].subject_kind, SubjectKind::Plan.as_str());
    assert_eq!(refs[0].subject_ref, plan_id().to_string());

    // 4. One undrained `PlanPublished`, carrying the pending handle.
    let events = outbox_rows(&h).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_name, "PlanPublished");
    assert_eq!(events[0].seq, 0);
    assert_eq!(events[0].published_at, None);
    assert_eq!(events[0].aggregate_id, plan_id().get());
    assert_eq!(
        events[0].payload.get("pendingVersionRef"),
        Some(&serde_json::json!("pend-0"))
    );
    // All three snapshot segments reach the stored row, not just the two the
    // commit could produce before D-162 gave the third a producer.
    assert_eq!(
        events[0].payload.get("evaluationPolicyVersion"),
        Some(&serde_json::json!(EVALUATION_POLICY_GENERATION))
    );

    // 5. One audit row, on the plan's own segment, after the seeds'.
    let records = publish_records(&h).await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].seq, SEEDED_AUTHORING_RECORDS);
    assert_eq!(records[0].entry_kind, "mutation");
    assert_eq!(records[0].segment_heads, None);
    assert_eq!(records[0].chain_id, plan_id().get());
    assert_eq!(records[0].actor_principal_id, ACTOR);
    assert_eq!(records[0].action, "publish");
    assert_eq!(records[0].subject_kind, "plan_revision");
    assert_eq!(records[0].approval_ref, None);
    assert_eq!(records[0].correlation_id, Some(CORRELATION));

    assert_seam_holds(&h).await;
}

#[tokio::test]
async fn an_approved_publish_puts_its_record_on_the_audit_trail() {
    let h = harness().await;
    let (revision, version, _) = seed_publishable(&h).await;
    let approval = Uuid::from_u128(0xa9_01);

    h.publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            PlanPublishUnit::new(plan_id(), revision),
            version,
            PublishAuthorization::approved(approval, Uuid::from_u128(1), Uuid::from_u128(2)),
            ACTOR,
            CORRELATION,
            at(12),
        )
        .await
        .expect("the publish commits");

    let records = publish_records(&h).await;
    assert_eq!(records[0].approval_ref, Some(approval));
}

#[tokio::test]
async fn a_second_publish_extends_every_counter_by_one() {
    let h = harness().await;
    let (revision, version, _) = seed_publishable(&h).await;
    h.publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            PlanPublishUnit::new(plan_id(), revision),
            version,
            PublishAuthorization::auto_publishable(),
            ACTOR,
            CORRELATION,
            at(12),
        )
        .await
        .expect("the first publish commits");

    let opened = h
        .plans
        .open_revision(&h.scope, TENANT, plan_id(), ACTOR, at(13))
        .await
        .expect("open the successor");
    h.publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            PlanPublishUnit::new(plan_id(), opened.revision),
            opened.row_version,
            PublishAuthorization::auto_publishable(),
            ACTOR,
            CORRELATION,
            at(14),
        )
        .await
        .expect("the second publish commits");

    // The predecessor is superseded and exactly one revision is current.
    let first = h
        .plans
        .find_revision(&h.scope, TENANT, plan_id(), revision)
        .await
        .expect("read the predecessor")
        .expect("it is there");
    assert_eq!(first.lifecycle_state, LifecycleState::Superseded);
    assert_eq!(
        h.plans
            .find_current(&h.scope, TENANT, plan_id())
            .await
            .expect("read the current revision")
            .expect("the plan has one")
            .revision,
        opened.revision
    );

    let mut events = outbox_rows(&h).await;
    events.sort_by_key(|row| row.seq);
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].seq, 1, "the aggregate's counter advanced by one");

    let records = publish_records(&h).await;
    assert_eq!(records.len(), 2);
    // The two publish records are **not** adjacent, and that is right: opening
    // the successor is itself an audited mutation, so its `create` record sits
    // between them on the plan's segment. What the chain guarantees is that every
    // row links to whatever preceded IT, which
    // `the_chain_verifies_across_a_mixed_sequence_of_authoring_and_publish_records`
    // walks in full. Here the claim is only that the second publish extended the
    // segment rather than restarting it.
    assert!(
        records[1].seq > records[0].seq,
        "the second publish extended the segment: {} then {}",
        records[0].seq,
        records[1].seq
    );
    assert!(
        records[1].prev_hash.is_some(),
        "and linked to whatever preceded it"
    );

    assert_eq!(version_refs(&h).await.len(), 2);
    assert_seam_holds(&h).await;
}

// ---------------------------------------------------------------------------
// The failure paths, and the claim this group exists to make.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_commit_time_validation_failure_writes_nothing_at_all() {
    // The world moved between approval and commit: the subject passed a
    // pre-check, and by the time the commit ran a second draft row had been
    // authored on a market with no covering phase row... in this construction,
    // a second row of the same plan on a class with no phase coverage. What
    // matters is that the second run finds it and that the transaction takes
    // everything back with it.
    let h = harness().await;
    let (revision, version, _) = seed_publishable(&h).await;

    // Passing before the world moves.
    let clean = h
        .publish
        .precheck(&h.scope, TENANT, plan_id(), at(11))
        .await
        .expect("the pre-check runs");
    assert!(
        clean.is_publishable(),
        "the subject was publishable: {clean:?}"
    );

    // The world moves: a row authored with no `roundingPolicyRef`, on a tenant
    // that has no default. Nothing about the approved content changed.
    h.prices
        .create_draft(
            &h.scope,
            TENANT,
            NewPriceDraft {
                price_id: Uuid::from_u128(0xb_0002),
                scope_key: scope_key(PriceEligibility::NewSubscriptionsOnly),
                content: PriceContent {
                    rounding_policy_ref: None,
                    ..flat_row()
                },
                created_by: ACTOR,
                created_at_utc: at(11),
            },
        )
        .await
        .expect("author the second row");

    let refusal = h
        .publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            PlanPublishUnit::new(plan_id(), revision),
            version,
            PublishAuthorization::auto_publishable(),
            ACTOR,
            CORRELATION,
            at(12),
        )
        .await
        .expect_err("the commit-time run must refuse");

    match &refusal {
        DomainError::ValidationFailed(report) => {
            assert!(
                report
                    .violations
                    .iter()
                    .any(|v| v.code == "ROUNDING_POLICY_UNRESOLVED"),
                "got {report:?}"
            );
        }
        other => panic!("expected a validation report, got {other:?}"),
    }

    // The subject is exactly where it was.
    let still = h
        .plans
        .find_revision(&h.scope, TENANT, plan_id(), revision)
        .await
        .expect("read the revision")
        .expect("it is there");
    assert_eq!(still.lifecycle_state, LifecycleState::Draft);
    assert_eq!(still.row_version, version, "its tag did not move either");

    // And nothing was written anywhere.
    assert!(version_refs(&h).await.is_empty());
    assert!(outbox_rows(&h).await.is_empty());
    assert!(
        publish_records(&h).await.is_empty(),
        "the refusal wrote no publish record; the authoring seeds' records are theirs"
    );
    assert_seam_holds(&h).await;
}

#[tokio::test]
async fn registry_absence_stops_the_publish_and_writes_nothing() {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    let provider = DBProvider::<DbError>::new(db);
    let h = Harness {
        plans: PlanRepo::new(provider.clone()),
        shapes: PlanShapeRepo::new(provider.clone()),
        prices: PriceRepo::new(provider.clone()),
        publish: PublishService::new(
            provider.clone(),
            &LimitsConfig::default(),
            FixtureGate::load(&committed_registry_path()),
            // The crate's real default, and the only registry it can have until
            // the registry gear exists.
            Arc::new(UnconfiguredCatalogVersionRegistryV1),
        ),
        registry: Arc::new(RegistryDouble::default()),
        scope: AccessScope::for_tenant(TENANT),
        provider,
    };
    let (revision, version, _) = seed_publishable(&h).await;

    let refusal = h
        .publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            PlanPublishUnit::new(plan_id(), revision),
            version,
            PublishAuthorization::auto_publishable(),
            ACTOR,
            CORRELATION,
            at(12),
        )
        .await
        .expect_err("a publish with no registry must stop");

    assert!(
        matches!(refusal, DomainError::CatalogVersionUnavailable(_)),
        "got {refusal:?}"
    );
    assert_eq!(
        h.plans
            .find_revision(&h.scope, TENANT, plan_id(), revision)
            .await
            .expect("read the revision")
            .expect("it is there")
            .lifecycle_state,
        LifecycleState::Draft
    );
    assert!(version_refs(&h).await.is_empty());
    assert!(outbox_rows(&h).await.is_empty());
    assert!(
        publish_records(&h).await.is_empty(),
        "the refusal wrote no publish record; the authoring seeds' records are theirs"
    );
    assert_seam_holds(&h).await;
}

#[tokio::test]
async fn a_stale_row_version_is_refused_and_writes_nothing() {
    let h = harness().await;
    let (revision, _, _) = seed_publishable(&h).await;

    let refusal = h
        .publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            PlanPublishUnit::new(plan_id(), revision),
            RowVersion::new(99),
            PublishAuthorization::auto_publishable(),
            ACTOR,
            CORRELATION,
            at(12),
        )
        .await
        .expect_err("a stale version must be refused");

    assert!(
        matches!(refusal, DomainError::StaleVersion(_)),
        "got {refusal:?}"
    );
    assert_eq!(
        h.plans
            .find_revision(&h.scope, TENANT, plan_id(), revision)
            .await
            .expect("read the revision")
            .expect("it is there")
            .lifecycle_state,
        LifecycleState::Draft
    );
    assert!(version_refs(&h).await.is_empty());
    assert!(outbox_rows(&h).await.is_empty());
    assert!(
        publish_records(&h).await.is_empty(),
        "the refusal wrote no publish record; the authoring seeds' records are theirs"
    );
}

#[tokio::test]
async fn a_retried_commit_re_requests_the_same_handle_rather_than_orphaning_one() {
    // The registry's own idempotency is what bounds the cost of holding the
    // request inside an open transaction, and the deterministic `request_id` is
    // what lets a caller use it.
    let registry = Arc::new(RegistryDouble::default());
    let h = harness_with(Arc::clone(&registry)).await;
    let (revision, version, _) = seed_publishable(&h).await;

    // A commit that fails after the request: the stale version is checked
    // inside `publish_revision`, which runs after the registry call.
    let _ = h
        .publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            PlanPublishUnit::new(plan_id(), revision),
            RowVersion::new(99),
            PublishAuthorization::auto_publishable(),
            ACTOR,
            CORRELATION,
            at(12),
        )
        .await
        .expect_err("the stale version refuses it");
    assert_eq!(h.registry.calls(), 1, "one handle was requested");

    h.publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            PlanPublishUnit::new(plan_id(), revision),
            version,
            PublishAuthorization::auto_publishable(),
            ACTOR,
            CORRELATION,
            at(12),
        )
        .await
        .expect("the retry commits");

    assert_eq!(
        h.registry.calls(),
        1,
        "the retry presented the same request_id and got the same handle back, \
         rather than orphaning the first at the registry"
    );
    assert_eq!(version_refs(&h).await.len(), 1);
}

#[tokio::test]
async fn a_plan_with_no_open_draft_revision_has_nothing_to_publish() {
    let h = harness().await;

    let refusal = h
        .publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            PlanPublishUnit::new(plan_id(), 0),
            RowVersion::new(0),
            PublishAuthorization::auto_publishable(),
            ACTOR,
            CORRELATION,
            at(12),
        )
        .await
        .expect_err("there is no draft revision");

    assert!(
        matches!(refusal, DomainError::NotFound { .. }),
        "got {refusal:?}"
    );
}

#[tokio::test]
async fn the_unit_must_name_the_revision_the_plan_actually_holds_open() {
    let h = harness().await;
    let (revision, version, _) = seed_publishable(&h).await;

    let refusal = h
        .publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            // A revision this plan does not hold open.
            PlanPublishUnit::new(plan_id(), revision + 7),
            version,
            PublishAuthorization::auto_publishable(),
            ACTOR,
            CORRELATION,
            at(12),
        )
        .await
        .expect_err("the unit must name the open draft");

    assert!(
        matches!(refusal, DomainError::NotFound { .. }),
        "got {refusal:?}"
    );
    assert!(version_refs(&h).await.is_empty());
}

#[tokio::test]
async fn a_failure_at_the_last_write_takes_the_four_before_it_back() {
    // The property this suite's module doc claims and the three refusal tests
    // above do **not** exercise: each of those refuses before the first write,
    // so their "nothing was written" assertions are true vacuously - there was
    // nothing to roll back. This one fails at step 6, the audit append, with the
    // revision flip, the price flip, the ref row and the outbox row already
    // issued inside the transaction.
    //
    // The failure is injected by putting the plan's audit segment into a state
    // `append` cannot extend: a head row whose `row_hash` is not 32 bytes, which
    // is an invariant breach the writer refuses rather than pads, because a
    // padded link is one the verification job reports as a break with no way
    // back to what was hashed.
    let h = harness().await;
    let (revision, version, price_id) = seed_publishable(&h).await;
    let conn = h.provider.conn().expect("conn");
    let corrupt = audit_log::ActiveModel {
        tenant_id: sea_orm::ActiveValue::Set(TENANT),
        chain_id: sea_orm::ActiveValue::Set(plan_id().get()),
        // Above the authoring seeds' records, so this row IS the head the
        // append will try to link to.
        seq: sea_orm::ActiveValue::Set(SEEDED_AUTHORING_RECORDS),
        entry_kind: sea_orm::ActiveValue::Set("mutation".to_owned()),
        recorded_at: sea_orm::ActiveValue::Set(at(9)),
        actor_principal_id: sea_orm::ActiveValue::Set(ACTOR),
        action: sea_orm::ActiveValue::Set("publish".to_owned()),
        subject_kind: sea_orm::ActiveValue::Set("plan_revision".to_owned()),
        subject_ref: sea_orm::ActiveValue::Set("plan/0".to_owned()),
        before_state: sea_orm::ActiveValue::Set(None),
        after_state: sea_orm::ActiveValue::Set(None),
        approval_ref: sea_orm::ActiveValue::Set(None),
        correlation_id: sea_orm::ActiveValue::Set(None),
        segment_heads: sea_orm::ActiveValue::Set(None),
        prev_hash: sea_orm::ActiveValue::Set(None),
        row_hash: sea_orm::ActiveValue::Set(vec![0_u8]),
    };
    audit_log::Entity::insert(corrupt.clone())
        .secure()
        .scope_with_model(&h.scope, &corrupt)
        .expect("scope the seeded head")
        .exec(&conn)
        .await
        .expect("seed a head the writer cannot link to");

    let refusal = h
        .publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            PlanPublishUnit::new(plan_id(), revision),
            version,
            PublishAuthorization::auto_publishable(),
            ACTOR,
            CORRELATION,
            at(12),
        )
        .await
        .expect_err("the audit append must refuse");
    assert!(
        matches!(refusal, DomainError::Internal(_)),
        "got {refusal:?}"
    );

    // The four writes that had already landed are gone.
    let still = h
        .plans
        .find_revision(&h.scope, TENANT, plan_id(), revision)
        .await
        .expect("read the revision")
        .expect("it is there");
    assert_eq!(
        still.lifecycle_state,
        LifecycleState::Draft,
        "the revision flip rolled back"
    );
    assert_eq!(still.row_version, version, "and so did its tag");
    assert_eq!(
        h.prices
            .find(&h.scope, TENANT, price_id)
            .await
            .expect("read the row")
            .expect("it is there")
            .lifecycle_state,
        LifecycleState::Draft,
        "the price flip rolled back"
    );
    assert!(
        version_refs(&h).await.is_empty(),
        "the pending ref rolled back"
    );
    assert!(outbox_rows(&h).await.is_empty(), "the event rolled back");
    assert_eq!(
        audit_rows(&h).await.len(),
        usize::try_from(SEEDED_AUTHORING_RECORDS).expect("a small count") + 1,
        "only the authoring seeds and the seeded head remain; the append never landed"
    );
    assert_seam_holds(&h).await;
}

#[tokio::test]
async fn a_row_authored_after_the_precheck_is_judged_by_the_second_run() {
    // §4.2's clause, from the other side. The pre-check is a pre-check: it does
    // not pin the subject, and the commit re-assembles and re-validates. So a
    // row authored between the two is **not** left behind — it is judged by the
    // second run, and publishes because it passed. The sibling case, where the
    // late row fails, is `a_commit_time_validation_failure_writes_nothing_at_all`.
    //
    // (The name used to promise the opposite and the body did neither. What
    // cannot be shown at this level is a row landing *inside* the commit's own
    // transaction — `sqlite::memory:` will not schedule a concurrent writer —
    // and that half is proven at the repository, where the validated set is an
    // argument: `a_row_authored_after_validation_is_not_published_by_this_commit`
    // in `tests/sqlite_price_repo.rs`.)
    let h = harness().await;
    let (revision, version, judged) = seed_publishable(&h).await;

    let clean = h
        .publish
        .precheck(&h.scope, TENANT, plan_id(), at(11))
        .await
        .expect("the pre-check runs");
    assert!(
        clean.is_publishable(),
        "the subject was publishable: {clean:?}"
    );

    // Authored after the pre-check said yes, and perfectly valid.
    let late = Uuid::from_u128(0xb_0002);
    h.prices
        .create_draft(
            &h.scope,
            TENANT,
            NewPriceDraft {
                price_id: late,
                scope_key: scope_key(PriceEligibility::NewSubscriptionsOnly),
                content: flat_row(),
                created_by: ACTOR,
                created_at_utc: at(11),
            },
        )
        .await
        .expect("author the late row");

    let receipt = h
        .publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            PlanPublishUnit::new(plan_id(), revision),
            version,
            PublishAuthorization::auto_publishable(),
            ACTOR,
            CORRELATION,
            at(12),
        )
        .await
        .expect("the publish commits");

    let mut published = receipt.published_price_ids().to_vec();
    published.sort();
    let mut both = vec![judged, late];
    both.sort();
    assert_eq!(
        published, both,
        "the second run judged the late row and published it"
    );
    for price_id in both {
        let row = h
            .prices
            .find(&h.scope, TENANT, price_id)
            .await
            .expect("read the row")
            .expect("it is there");
        assert_eq!(row.lifecycle_state, LifecycleState::Published);
        assert_eq!(
            row.row_version,
            RowVersion::new(0),
            "publishing changes no content, so the tag it was validated at is the tag it keeps"
        );
    }
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

// ---------------------------------------------------------------------------
// The vocabulary and the chain, over every writer the crate has (D-135, D-158).
// ---------------------------------------------------------------------------

/// Drive **every** audited path this crate has, on one plan, and return the
/// segment's rows in seq order.
///
/// One plan on purpose: D-135 keys the segment on the audited subject's
/// aggregate, so the whole sequence extends one chain and the walk below is the
/// walk a verification job makes.
async fn drive_every_audited_path(h: &Harness) -> Vec<audit_log::Model> {
    // create (plan), update x2 (facets), create (price)
    let (revision, version, _) = seed_publishable(h).await;
    // publish
    h.publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            PlanPublishUnit::new(plan_id(), revision),
            version,
            PublishAuthorization::auto_publishable(),
            ACTOR,
            CORRELATION,
            at(12),
        )
        .await
        .expect("the publish commits");

    // A second draft price row, authored and then discarded: `delete`.
    let doomed = Uuid::from_u128(0xb_0002);
    h.prices
        .create_draft(
            &h.scope,
            TENANT,
            NewPriceDraft {
                price_id: doomed,
                scope_key: scope_key(PriceEligibility::NewSubscriptionsOnly),
                content: flat_row(),
                created_by: ACTOR,
                created_at_utc: at(13),
            },
        )
        .await
        .expect("author a second row");
    h.prices
        .delete_draft(&h.scope, TENANT, doomed, RowVersion::new(0), stamp())
        .await
        .expect("discard it");

    // A successor revision, opened and then discarded: `abandon`.
    let opened = h
        .plans
        .open_revision(&h.scope, TENANT, plan_id(), ACTOR, at(14))
        .await
        .expect("open a successor");
    h.plans
        .abandon_draft(
            &h.scope,
            TENANT,
            plan_id(),
            opened.revision,
            opened.row_version,
            stamp(),
        )
        .await
        .expect("discard it");

    let mut rows = audit_rows(h).await;
    rows.sort_by_key(|row| row.seq);
    rows
}

#[tokio::test]
async fn every_declared_action_has_a_production_writer() {
    // D-158's constraint, as a guard rather than a promise: **a token with no
    // writer is not declared**, because a vocabulary entry nobody writes reads as
    // coverage to everyone who greps for it. This walks `AuditAction::ALL` and
    // insists each one is produced by driving the crate's own paths.
    //
    // Delete a writer and this fails; add a token without one and this fails.
    // Those are the two directions the vocabulary can drift in.
    let h = harness().await;
    let rows = drive_every_audited_path(&h).await;

    let written: std::collections::BTreeSet<String> =
        rows.iter().map(|row| row.action.clone()).collect();
    let declared: std::collections::BTreeSet<String> = AuditAction::ALL
        .iter()
        .map(|action| action.as_str().to_owned())
        .collect();

    assert_eq!(
        written, declared,
        "every declared action has a writer, and every writer's action is declared"
    );

    // The same, one column over.
    let kinds: std::collections::BTreeSet<String> =
        rows.iter().map(|row| row.subject_kind.clone()).collect();
    let declared_kinds: std::collections::BTreeSet<String> = AuditSubjectKind::ALL
        .iter()
        .map(|kind| kind.as_str().to_owned())
        .collect();
    assert_eq!(kinds, declared_kinds);
}

#[tokio::test]
async fn the_chain_verifies_across_a_mixed_sequence_of_authoring_and_publish_records() {
    // The property a chain exists for, over the writers this group added. Every
    // row's digest is recomputed from the columns that stored it and compared to
    // the `row_hash` the writer put there - a test that only asserted "a hash was
    // written" would keep passing after the encoding and the store had stopped
    // agreeing, and a new writer is exactly the occasion for that.
    let h = harness().await;
    let rows = drive_every_audited_path(&h).await;
    assert!(rows.len() >= 7, "the sequence is mixed: {}", rows.len());

    let mut prev = genesis_prev_hash(TENANT, plan_id().get());
    for (position, row) in rows.iter().enumerate() {
        assert_eq!(
            u64::try_from(position).expect("a small position"),
            u64::try_from(row.seq).expect("a non-negative seq"),
            "the segment has no gaps"
        );
        assert_eq!(
            row.prev_hash.as_deref(),
            Some(prev.as_slice()),
            "row {} links to its predecessor",
            row.seq
        );
        let action = AuditAction::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.as_str() == row.action)
            .expect("a declared action");
        let kind = AuditSubjectKind::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.as_str() == row.subject_kind)
            .expect("a declared subject kind");
        let record = AuditRecord {
            tenant_id: row.tenant_id,
            chain_id: row.chain_id,
            seq: u64::try_from(row.seq).expect("a non-negative seq"),
            recorded_at: row.recorded_at,
            actor_principal_id: row.actor_principal_id,
            action,
            subject_kind: kind,
            subject_ref: &row.subject_ref,
            before_state: row.before_state.as_ref(),
            after_state: row.after_state.as_ref(),
            approval_ref: row.approval_ref,
            correlation_id: row.correlation_id,
        };
        let recomputed = audit_row_hash(&record, &prev).expect("recompute the digest");
        assert_eq!(
            recomputed.as_slice(),
            row.row_hash.as_slice(),
            "row {} reproduces its own digest",
            row.seq
        );
        prev = recomputed;
    }
}

// ---------------------------------------------------------------------------
// A same-aggregate contention is tellable from a dead connection (D-159)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_loser_at_the_outbox_sequence_is_a_concurrent_mutation_and_not_a_storage_fault() {
    // D-159's second serialization point, and the one whose unique violation a
    // single-writer suite can actually provoke. Before it, the loser reached the
    // caller as `RepoError::Db` -> `DomainError::Internal` -> **500**,
    // indistinguishable from a dead connection, for a request whose entire remedy
    // is to retry.
    //
    // The violation here is `uq_pricing_outbox_dedup_key`'s rather than
    // `uq_pricing_outbox_sequence`'s, and the driver's class does not tell them
    // apart. Both mean the same thing - another write of this aggregate got here
    // first - and both remedy the same way; the constraint **names** are what a
    // Postgres suite would assert.
    let h = harness().await;
    let payload = PlanPublishedPayload {
        plan_id: plan_id(),
        revision: 0,
        pending_version_ref: "pend-0".to_owned(),
        price_ids: Vec::new(),
        correlation_id: CORRELATION,
    };

    let (_, first) = h
        .provider
        .db()
        .in_transaction::<u64, bss_pricing::infra::storage::RepoError, _>(|txn| {
            let scope = AccessScope::for_tenant(TENANT);
            let event = NewOutboxEvent::plan_published(TENANT, &payload, at(12));
            Box::pin(async move { outbox_repo::enqueue(txn, &scope, event).await })
        })
        .await;
    first.expect("the first enqueue lands");

    let (_, second) = h
        .provider
        .db()
        .in_transaction::<u64, bss_pricing::infra::storage::RepoError, _>(|txn| {
            let scope = AccessScope::for_tenant(TENANT);
            let event = NewOutboxEvent::plan_published(TENANT, &payload, at(13));
            Box::pin(async move { outbox_repo::enqueue(txn, &scope, event).await })
        })
        .await;
    let refusal = second.expect_err("the second is refused by a unique index");
    let refusal = refusal.into_domain(|infra| {
        bss_pricing::infra::storage::RepoError::Db(format!("outbox transaction: {infra}"))
    });

    assert!(
        matches!(
            refusal,
            bss_pricing::infra::storage::RepoError::ConcurrentMutation { .. }
        ),
        "the loser is a contention, not a storage fault: {refusal:?}"
    );
    assert!(
        refusal.to_string().contains(&plan_id().to_string()),
        "and it names the aggregate to retry against: {refusal}"
    );

    // The wire answer: 409 with its own code, never a 500.
    let canonical = toolkit::api::canonical_prelude::CanonicalError::from(
        bss_pricing::infra::storage::repo_failure(&refusal),
    );
    assert_eq!(canonical.status_code(), 409, "{canonical:?}");
    assert!(
        format!("{canonical:?}").contains("CONCURRENT_MUTATION"),
        "{canonical:?}"
    );
}

#[tokio::test]
async fn the_publish_record_carries_the_before_and_after_state_its_transition_implies() {
    // The third before/after writer, and the one the existing suite could not
    // see. Blanking `before_state`/`after_state` to `None` in `infra/publish.rs`
    // left 846/846 green - because the only readers of this pair are the two
    // digest-recompute walks, which rebuild each record's preimage **from the
    // stored columns** and are therefore self-consistent under any substitution.
    // A walk that derives its expectation from the data it is checking cannot
    // catch the data being replaced.
    //
    // So this asserts the CONTENT against values held independently of the row:
    // the version the publish was submitted against, the state the flip produced,
    // and the pending handle. Blank the pair and exactly this test fails.
    let h = harness().await;
    let (revision, version, _) = seed_publishable(&h).await;
    h.publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            PlanPublishUnit::new(plan_id(), revision),
            version,
            PublishAuthorization::auto_publishable(),
            ACTOR,
            CORRELATION,
            at(12),
        )
        .await
        .expect("the publish commits");

    let records = publish_records(&h).await;
    assert_eq!(records.len(), 1);
    let before = records[0]
        .before_state
        .as_ref()
        .expect("a publish records what it moved from");
    let after = records[0]
        .after_state
        .as_ref()
        .expect("and what it moved to");

    // The draft the commit judged, at the version the caller's precondition
    // matched - `version` comes from the seed, not from this row.
    assert_eq!(
        before.get("lifecycleState"),
        Some(&serde_json::json!("draft"))
    );
    assert_eq!(
        before.get("rowVersion"),
        Some(&serde_json::json!(version.get()))
    );

    // The state the flip produced, one version on.
    assert_eq!(
        after.get("lifecycleState"),
        Some(&serde_json::json!("published"))
    );
    assert_eq!(
        after.get("rowVersion"),
        Some(&serde_json::json!(version.get() + 1))
    );

    // The one thing this record carries that no other audit record does: the
    // pending handle. Without it an auditor has the flip and no way to reach the
    // version it landed in, which is the whole reason `subject_state` takes a
    // `pending_ref` at all.
    assert_eq!(
        after.get("pendingVersionRef"),
        Some(&serde_json::json!("pend-0"))
    );
    assert!(
        before.get("pendingVersionRef").is_none(),
        "and the before-state has none: there was no handle before the commit \
         requested one: {before}"
    );
}
