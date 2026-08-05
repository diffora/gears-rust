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

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bss_pricing::config::LimitsConfig;
use bss_pricing::domain::approval::{ApprovalState, DecisionBy};
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
use bss_pricing::infra::approval::{ApprovalService, DecideRequest, RegionGrant};
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

/// One value for a whole test binary: these suites drive a repository or a
/// service directly, where the value the HTTP edge would have established has
/// no producer. What each suite asserts *about* it is stated where it asserts
/// it.
const TEST_CORRELATION: uuid::Uuid = uuid::Uuid::from_u128(0x_c0_11_a7_10);

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
/// The independent second principal. Distinct from [`ACTOR`], which is what
/// `chk_pricing_approval_distinct_principals` and `inst-tp-distinct` both
/// require of an approve.
const APPROVER: Uuid = Uuid::from_u128(0xac_11);

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
        correlation_id: TEST_CORRELATION,
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
                correlation_id: TEST_CORRELATION,
            },
        )
        .await
        .expect("author the price row");

    // `inst-wc-required`: the row does not publish until its canonical scope key
    // holds an active or scheduled window. Thirteen tests in this file reddened
    // when the rule registered, every one of them because the plan they publish
    // had no window at all.
    //
    // The interval is `common::schedule_coverage_window`'s, not this file's, so
    // the seed cannot drift from the three other suites that owe the same thing.
    let conn = h.provider.conn().expect("conn");
    common::schedule_coverage_window(&conn, &h.scope, TENANT, price_id, stamp()).await;

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
            PlanPublishUnit::plan_content(plan_id(), revision),
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

    // The authorization is minted from a **real** decided record rather than
    // assembled by hand, and since 2026-08-04 it has to be: `Approved` carries
    // the digest the decision was over, and the commit re-derives it inside its
    // own transaction. A hand-built pin would be a pin over nothing.
    let record = approve_unit(&h, approval).await;

    h.publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            PlanPublishUnit::plan_content(plan_id(), revision),
            version,
            authorization_of(&record),
            ACTOR,
            CORRELATION,
            at(12),
        )
        .await
        .expect("the publish commits");

    let records = publish_records(&h).await;
    assert_eq!(records[0].approval_ref, Some(approval));
}

/// The publish commit refuses content the decision was not over
/// (`inst-ap-pin`'s approve→commit window).
///
/// **Staged with the world held still and one byte of the pin moved**, which is
/// what makes the digest the only variable: same plan, same revision, same rows,
/// same `row_version` everywhere, and an authorization carrying a pin that no
/// longer matches what the commit re-derives. Staging a *mutation* instead would
/// move a `row_version`, and the compare-and-swap would answer `STALE_VERSION`
/// whether or not the pin is bound to anything — which is the trap this staging
/// exists to avoid.
///
/// **This doc used to describe a different staging** — a unit opened over one
/// draft and a commit aimed at a second plan's identically-shaped draft — which
/// the body never performed. The prose was left over from an abandoned attempt;
/// the behaviour proved is the stronger of the two, because a corrupted pin
/// isolates the check from every other guard in the commit.
///
/// # What this does **not** exercise, said plainly
///
/// The in-transaction re-derivation is driven here only against a **corrupted
/// authorization**, never against a world that actually moved between the
/// decision and the commit. There is no test anywhere that stages the latter,
/// and the reason is structural rather than an omission of effort: the publish
/// route looks an approval up by subject **and content**
/// (`approval_repo::find_approved_for_content`), so a subject that moved after
/// the approve has no approved unit and never reaches `commit` at all — that
/// path is `rest_publish::a_row_edited_after_the_approve_does_not_publish_under_
/// the_stale_decision`, and it ends in a fresh 202. The only way to reach the
/// re-derivation with a genuinely moved world is the approve→commit race, where
/// the mutation lands after the route's lookup and before the commit's
/// transaction, and nothing in this crate stages it. What is proved here is that
/// the check is bound to the digest; that it is bound to the *current* world is
/// carried by `infra::publish`'s placement of the re-derivation inside the
/// commit's own transaction, and by nothing executable.
#[tokio::test]
async fn a_commit_whose_content_is_not_what_was_approved_is_refused() {
    let h = harness().await;
    let (revision, version, _) = seed_publishable(&h).await;
    let record = approve_unit(&h, Uuid::from_u128(0xa9_02)).await;

    // The same authorization, with one byte of the pin moved. Nothing else in
    // the world changes — same revision, same version, same rows.
    let mut moved = record.content_hash.clone();
    moved[0] ^= 0xff;
    let stale = PublishAuthorization::approved(
        record.approval_id,
        record.submitter_principal,
        record.approver_principal.expect("an approved record"),
        moved.as_slice().try_into().expect("a 32-byte digest"),
    );

    let refused = h
        .publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            PlanPublishUnit::plan_content(plan_id(), revision),
            version,
            stale,
            ACTOR,
            CORRELATION,
            at(12),
        )
        .await
        .expect_err("the content approved is not the content this would freeze");
    assert!(
        matches!(refused, DomainError::ApprovalContentMismatch(_)),
        "got {refused:?}"
    );

    // And nothing was frozen: the refusal is ahead of every write.
    assert_eq!(
        h.plans
            .find_revision(&h.scope, TENANT, plan_id(), revision)
            .await
            .expect("read the revision")
            .expect("it is there")
            .lifecycle_state,
        LifecycleState::Draft
    );
}

/// Open a unit over the seeded plan and approve it as an independent principal.
async fn approve_unit(
    h: &Harness,
    approval_id: Uuid,
) -> bss_pricing::infra::storage::repo::approval_repo::ApprovalRecord {
    let approvals = ApprovalService::new(h.provider.clone());
    approvals
        .submit(
            &h.scope,
            TENANT,
            plan_id(),
            approval_id,
            serde_json::json!({ "material": true, "reason": "noConfiguredThreshold" }),
            stamp_of(ACTOR, at(11)),
        )
        .await
        .expect("open the pending unit");
    approvals
        .decide(
            &h.scope,
            TENANT,
            DecideRequest {
                approval_id,
                decision: DecisionBy::Approve(APPROVER),
                reason: None,
                approver_regions: RegionGrant::Explicit(std::collections::BTreeSet::from([
                    Region::new("eu").expect("a non-blank region"),
                ])),
                stamp: stamp_of(APPROVER, at(11)),
            },
        )
        .await
        .expect("an independent principal approves it")
}

/// The authorization a decided record yields, spelled once for this suite.
fn authorization_of(
    record: &bss_pricing::infra::storage::repo::approval_repo::ApprovalRecord,
) -> PublishAuthorization {
    PublishAuthorization::approved(
        record.approval_id,
        record.submitter_principal,
        record.approver_principal.expect("an approved record"),
        record
            .content_hash
            .as_slice()
            .try_into()
            .expect("a 32-byte digest"),
    )
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
            PlanPublishUnit::plan_content(plan_id(), revision),
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
        .open_revision(&h.scope, TENANT, plan_id(), stamp_of(ACTOR, at(13)))
        .await
        .expect("open the successor");
    h.publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            PlanPublishUnit::plan_content(plan_id(), opened.revision),
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
                correlation_id: TEST_CORRELATION,
            },
        )
        .await
        .expect("author the second row");

    // The late row gets its coverage window too, so the commit-time refusal is
    // the **one** this test names. Without it the row is unpublishable for two
    // reasons at once, and an `any(code == ...)` assertion would stay green with
    // the rounding rule deleted — the second fault answering for the first.
    common::schedule_coverage_window(
        &h.provider.conn().expect("conn"),
        &h.scope,
        TENANT,
        Uuid::from_u128(0xb_0002),
        stamp(),
    )
    .await;

    let refusal = h
        .publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            PlanPublishUnit::plan_content(plan_id(), revision),
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
            let codes: Vec<&str> = report.violations.iter().map(|v| v.code.as_str()).collect();
            assert_eq!(
                codes,
                ["ROUNDING_POLICY_UNRESOLVED"],
                "exactly the fault this test staged, and nothing standing in for it"
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
            PlanPublishUnit::plan_content(plan_id(), revision),
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
            PlanPublishUnit::plan_content(plan_id(), revision),
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
            PlanPublishUnit::plan_content(plan_id(), revision),
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
            PlanPublishUnit::plan_content(plan_id(), revision),
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
            PlanPublishUnit::plan_content(plan_id(), 0),
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
            PlanPublishUnit::plan_content(plan_id(), revision + 7),
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
            PlanPublishUnit::plan_content(plan_id(), revision),
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
                correlation_id: TEST_CORRELATION,
            },
        )
        .await
        .expect("author the late row");

    // The late row is on a **different** canonical scope key
    // (`new_subscriptions_only`), so `inst-wc-perkey` gives it its own coverage
    // obligation: covering the seed's key covers nothing else. This test is about
    // the late row *publishing*, so it has to be publishable.
    common::schedule_coverage_window(
        &h.provider.conn().expect("conn"),
        &h.scope,
        TENANT,
        late,
        stamp(),
    )
    .await;

    let receipt = h
        .publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            PlanPublishUnit::plan_content(plan_id(), revision),
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
        correlation_id: TEST_CORRELATION,
    }
}

/// The stamp a decision is taken under: who acted, when, and the request's
/// correlation.
fn stamp_of(
    actor: uuid::Uuid,
    when: chrono::DateTime<chrono::Utc>,
) -> bss_pricing::domain::audit::AuditStamp {
    bss_pricing::domain::audit::AuditStamp {
        actor_principal_id: actor,
        recorded_at: when,
        correlation_id: TEST_CORRELATION,
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
/// The approval plane's five audited paths, on the plan the caller has already
/// driven the authoring ones over.
///
/// Split out of [`drive_every_audited_path`] because the two planes are two
/// stories and one function telling both is what `clippy::cognitive_complexity`
/// is measuring; the segment is the same either way, which is the point of the
/// caller.
async fn drive_the_approval_plane(h: &Harness) {
    // A self-approval attempt, refused: `deny`. It is the one audited path whose
    // record is **not** a mutation - see `AuditAction::Deny` - and it is driven
    // here rather than only in `tests/sqlite_approval_service.rs` because this
    // is the test that holds the vocabulary and its writers to each other.
    //
    // The unit is opened over the plan's *current* draft revision, which the
    // abandon above consumed, so a fresh one is opened first.
    h.plans
        .open_revision(&h.scope, TENANT, plan_id(), stamp_of(ACTOR, at(15)))
        .await
        .expect("open a revision to submit");
    let approvals = ApprovalService::new(h.provider.clone());
    let approval_id = Uuid::from_u128(0xa_0001);
    approvals
        .submit(
            &h.scope,
            TENANT,
            plan_id(),
            approval_id,
            serde_json::json!({ "reason": "noConfiguredThreshold" }),
            stamp_of(ACTOR, at(15)),
        )
        .await
        .expect("open the pending unit");
    let refused = approvals
        .decide(
            &h.scope,
            TENANT,
            DecideRequest {
                approval_id,
                // The submitter, which is what makes it a violation.
                decision: DecisionBy::Approve(ACTOR),
                reason: None,
                approver_regions: RegionGrant::Explicit(std::collections::BTreeSet::from([
                    Region::new("eu").expect("a non-blank region"),
                ])),
                stamp: stamp_of(ACTOR, at(16)),
            },
        )
        .await
        .expect_err("the submitter may not approve their own unit");
    assert!(
        matches!(refused, DomainError::SelfApprovalForbidden(_)),
        "got {refused:?}"
    );

    // The three decisions an independent reviewer can take, each on its own
    // unit: `approve`, `reject`, `withdraw`. They are driven in sequence rather
    // than in parallel because `inst-co-single-pending` allows a plan **one**
    // pending unit at a time, so each has to be decided before the next opens -
    // which is also why each `submit` below lands its own `submit` record.
    let approved = decide_one(
        h,
        &approvals,
        approval_id,
        DecisionBy::Approve(APPROVER),
        None,
    )
    .await;
    assert_eq!(approved.state, ApprovalState::Approved);

    let rejected_id = Uuid::from_u128(0xa_0002);
    open_unit(h, &approvals, rejected_id).await;
    let rejected = decide_one(
        h,
        &approvals,
        rejected_id,
        DecisionBy::Reject(APPROVER),
        Some("margin below floor".to_owned()),
    )
    .await;
    assert_eq!(rejected.state, ApprovalState::Rejected);

    let withdrawn_id = Uuid::from_u128(0xa_0003);
    open_unit(h, &approvals, withdrawn_id).await;
    let withdrawn = decide_one(
        h,
        &approvals,
        withdrawn_id,
        // The **submitter's own** withdraw, which is the case `inst-as-void`
        // names and the one whose actor `approver_principal` cannot hold.
        DecisionBy::Void(Some(ACTOR)),
        Some("superseded by a later change set".to_owned()),
    )
    .await;
    assert_eq!(withdrawn.state, ApprovalState::Voided);
}

/// Open one pending unit over the plan's current draft revision.
async fn open_unit(h: &Harness, approvals: &ApprovalService, approval_id: Uuid) {
    approvals
        .submit(
            &h.scope,
            TENANT,
            plan_id(),
            approval_id,
            serde_json::json!({ "reason": "noConfiguredThreshold" }),
            stamp_of(ACTOR, at(15)),
        )
        .await
        .expect("open the pending unit");
}

/// Schedule a window through the service, so the `window` subject kind has a writer.
///
/// The plan is already `published` by the time this runs, which is the ordinary
/// subject of a window mutation and the reason the service resolves the plan's
/// **current** revision rather than an open draft.
/// The D-10 threshold-policy proposal — the writer of the `policy` subject kind.
///
/// It is here rather than in a suite of its own because this file owns the
/// **vocabulary** property: `every_declared_action_has_a_production_writer` walks
/// what driving the crate's own paths actually wrote, and a token whose writer is
/// never driven is indistinguishable from a token with no writer. Adding
/// `AuditSubjectKind::Policy` without this line fails that test, which is the
/// direction D-158 asks the guard to fail in.
///
/// The proposal writes a `submit` record on the **policy** chain, not on any plan's
/// — `audit_repo::policy_chain`, a per-tenant segment — so it also exercises the
/// only subject kind whose aggregate is not a plan.
async fn drive_the_threshold_policy_plane(h: &Harness) {
    let thresholds = bss_pricing::infra::threshold::ThresholdService::new(h.provider.clone());
    let unit = Uuid::now_v7();
    thresholds
        .propose(
            &h.scope,
            TENANT,
            unit,
            at(16),
            vec![bss_pricing::domain::materiality::ThresholdEntry {
                currency: bss_pricing::domain::money::CurrencyCode::new("EUR")
                    .expect("a valid code"),
                basis: bss_pricing::domain::materiality::ThresholdBasis::Absolute { minor: 500 },
            }],
            serde_json::json!({ "material": true, "reason": "alwaysMaterialTrigger" }),
            stamp_of(ACTOR, at(16)),
        )
        .await
        .expect("the proposal opens its unit");
    // **And an independent principal approves it**, which is what makes it the
    // tenant's policy (D-10). Not decoration for this census: the window plane below
    // needs a configured entry for `EUR` or its mutation is material and writes
    // nothing. `approver_regions` is the explicit grant the scope rule measures
    // against, and a policy unit's change set touches no region at all.
    ApprovalService::new(h.provider.clone())
        .decide(
            &h.scope,
            TENANT,
            DecideRequest {
                approval_id: unit,
                decision: DecisionBy::Approve(APPROVER),
                reason: None,
                approver_regions: RegionGrant::Explicit(std::collections::BTreeSet::from([
                    Region::new("eu").expect("a non-blank region"),
                ])),
                stamp: stamp_of(APPROVER, at(17)),
            },
        )
        .await
        .expect("an independent principal puts the policy in force");
}

async fn drive_the_window_plane(h: &Harness) {
    let windows = bss_pricing::infra::window::WindowService::new(
        h.provider.clone(),
        Arc::clone(&h.registry) as Arc<dyn CatalogVersionRegistryV1>,
    );
    let price_id = h
        .prices
        .list_for_plan(&h.scope, TENANT, plan_id(), &[LifecycleState::Published])
        .await
        .expect("read the published rows")
        .first()
        .expect("the commit above published one")
        .price_id;
    windows
        .schedule(
            &ctx(),
            &h.scope,
            TENANT,
            price_id,
            Uuid::now_v7(),
            // **Exactly** where the plan's seeded window ends, which is legal and
            // deliberate on two counts: the intervals are half-open, so
            // `effectiveTo == next.effectiveFrom` is adjacency and not an overlap,
            // and it therefore opens no interior gap for `inst-fg-detect` either.
            // Scheduling anywhere earlier collides with that window; anywhere later
            // opens a hole, and either would make this census fail for a reason
            // that has nothing to do with the vocabulary it is about.
            //
            // 2099 is a fact rather than a date off the clock: a window dated today
            // races the activation sweep, which is a defect this program has already
            // paid for once.
            Utc.with_ymd_and_hms(2099, 9, 1, 0, 0, 0)
                .single()
                .expect("a real instant"),
            None,
            "audited-window-writer".to_owned(),
            bss_pricing::api::rest::windows::verdict_json,
            stamp_of(ACTOR, at(18)),
        )
        .await
        .expect("schedule a window as a publish unit");
}

/// Take one decision on a pending unit, as an in-scope reviewer.
async fn decide_one(
    h: &Harness,
    approvals: &ApprovalService,
    approval_id: Uuid,
    decision: DecisionBy,
    reason: Option<String>,
) -> bss_pricing::infra::storage::repo::approval_repo::ApprovalRecord {
    let actor = decision
        .decider()
        .expect("a human decision names its actor");
    approvals
        .decide(
            &h.scope,
            TENANT,
            DecideRequest {
                approval_id,
                decision,
                reason,
                approver_regions: RegionGrant::Explicit(std::collections::BTreeSet::from([
                    Region::new("eu").expect("a non-blank region"),
                ])),
                stamp: stamp_of(actor, at(17)),
            },
        )
        .await
        .expect("the decision is taken")
}

async fn drive_every_audited_path(h: &Harness) -> Vec<audit_log::Model> {
    // create (plan), update x2 (facets), create (price)
    let (revision, version, _) = seed_publishable(h).await;
    // publish
    h.publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            PlanPublishUnit::plan_content(plan_id(), revision),
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
                correlation_id: TEST_CORRELATION,
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
        .open_revision(&h.scope, TENANT, plan_id(), stamp_of(ACTOR, at(14)))
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

    // The approval plane's five: `submit`, `deny`, `approve`, `reject`, `withdraw`.
    drive_the_approval_plane(h).await;

    // Slice 7's window plane, which writes the `window` subject kind. It is driven
    // through `WindowService` and not through the repository, because the record is
    // the *service's* — `window_repo` takes an `AuditStamp` and deliberately writes
    // no row, and a driver that reached past the service would leave this census
    // green with no window record in the store at all.
    //
    // The action it writes is `publish`, already covered by the plan commit above;
    // what this adds to the census is the **subject kind**, which is the half that
    // was failing. `inst-ws-publishunit` is the warrant: a window mutation runs the
    // Foundation §4.2 engine path, and S5 §6 declares `publish` against §4.2.
    // **The policy plane runs first, and the order is now load-bearing.** A window
    // schedule is not on `inst-mat-registered`'s trigger list, so what decides it is
    // the tenant's threshold policy: with none configured, `inst-mat-failsafe` answers,
    // the mutation writes **nothing at all** and this census loses the `window` subject
    // kind entirely. So the policy is proposed *and approved* before the window plane
    // is driven — which is also the operator sequence D-10 forces.
    drive_the_threshold_policy_plane(h).await;
    drive_the_window_plane(h).await;

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

    // **Two segments, not one**, since the threshold-policy proposal and its approval
    // joined the driven set. D-135 keys a segment on the audited subject's *aggregate* and a
    // policy's is the tenant's policy rather than any plan, so its record opens a
    // chain of its own at `seq 0` — which a single walk over every row reads as a
    // gap in the plan's segment. Partitioning is not a workaround for that: the
    // partition **is** the property, and its sizes are asserted so that a policy
    // record silently filed on a plan's chain fails here rather than verifying
    // perfectly on the wrong segment.
    let (plan_rows, policy_rows): (Vec<_>, Vec<_>) = rows
        .iter()
        .cloned()
        .partition(|row| row.chain_id == plan_id().get());
    assert_eq!(
        policy_rows.len(),
        2,
        "the proposal and its approval, both on the policy segment - the approve joined the \
         driven set because the window plane below it needs a policy that is actually in force"
    );
    assert_eq!(
        policy_rows[0].chain_id,
        bss_pricing::infra::storage::repo::audit_repo::policy_chain(),
        "and that segment is the policy chain, never a plan's"
    );
    verify_segment(&plan_rows, plan_id().get());
    verify_segment(&policy_rows, policy_rows[0].chain_id);
}

/// Walk one segment link by link and recompute every row's digest.
///
/// Extracted when the driven set grew a second aggregate. It takes the `chain_id`
/// rather than reading it off the first row on purpose: the genesis digest is a
/// function of `(tenant, chain)`, so a helper that derived it from the rows it was
/// checking would verify any segment against itself.
fn verify_segment(rows: &[audit_log::Model], chain_id: Uuid) {
    let mut prev = genesis_prev_hash(TENANT, chain_id);
    for (position, row) in rows.iter().enumerate() {
        assert_eq!(row.chain_id, chain_id, "the segment holds one chain");
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

/// **Two publish records on one segment, walked link by link.**
///
/// The combination Phase 2 recorded and could not close: it had a link-by-link
/// walk over a segment holding **one** publish record
/// (`the_chain_verifies_across_a_mixed_sequence_of_authoring_and_publish_records`)
/// and a two-publish test that asserted extension without walking
/// (`a_second_publish_extends_every_counter_by_one`). Neither covers a chain
/// that has to stay connected **across** two freezes of one plan, and the second
/// publish is the first writer that appends to a segment a previous publish
/// already extended.
///
/// # This is the connectedness half, and it is deliberately blind to content
///
/// It asserts one property — *the segment is a chain* — and nothing about what
/// any row holds. It does **not** recompute a single digest, and that is the
/// point rather than an omission: `audit_row_hash` over a preimage rebuilt from
/// the stored columns is self-consistent by construction, so a writer that
/// blanked `before_state` and `after_state` would store NULLs, hash NULLs, and
/// agree with itself. The content half is owned by the test above (per-row
/// recompute) and, from an expectation built outside the store,
/// `postgres_audit_chain::every_row_holds_the_record_its_writer_was_handed_and_hashes_to_it`.
/// Letting one test carry both is how a suite ends up unable to tell a broken
/// link from a dropped field, because whichever assertion fires first hides the
/// other.
///
/// The two independent facts it does bring from outside the segment are the
/// genesis seed — computed from the tenant and the chain, never read from a
/// column — and the distinctness of the digests, without which a writer storing
/// one constant `row_hash` everywhere would satisfy an adjacency walk from row 1
/// on.
#[tokio::test]
async fn a_segment_holding_two_publishes_is_one_unbroken_chain() {
    let h = harness().await;

    // The first freeze, under a real approval: the two-person rule is what makes
    // a publish record's `approval_ref` non-null, and a segment whose publishes
    // were both auto-publishable would not be the segment a governed deployment
    // ever holds.
    let (revision, version, _) = seed_publishable(&h).await;
    let first_unit = approve_unit(&h, Uuid::from_u128(0xa_7001)).await;
    h.publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            PlanPublishUnit::plan_content(plan_id(), revision),
            version,
            authorization_of(&first_unit),
            ACTOR,
            CORRELATION,
            at(12),
        )
        .await
        .expect("the first publish commits");

    // The successor, opened and frozen in turn. Opening it is itself an audited
    // mutation, so the two publish records are **not** adjacent - which is
    // exactly why the walk has to be a walk rather than a comparison of the two.
    let opened = h
        .plans
        .open_revision(&h.scope, TENANT, plan_id(), stamp_of(ACTOR, at(13)))
        .await
        .expect("open the successor");
    let second_unit = approve_unit(&h, Uuid::from_u128(0xa_7002)).await;
    h.publish
        .commit(
            &ctx(),
            &h.scope,
            TENANT,
            PlanPublishUnit::plan_content(plan_id(), opened.revision),
            opened.row_version,
            authorization_of(&second_unit),
            ACTOR,
            CORRELATION,
            at(14),
        )
        .await
        .expect("the second publish commits");

    let mut rows = audit_rows(&h).await;
    rows.sort_by_key(|row| row.seq);

    // The world is the one this test claims: one segment, two freezes, and
    // something between them. Without this the walk below could be green over a
    // segment with one publish record, or none.
    let publishes: Vec<i64> = rows
        .iter()
        .filter(|row| row.action == AuditAction::Publish.as_str())
        .map(|row| row.seq)
        .collect();
    assert_eq!(
        publishes.len(),
        2,
        "the segment must hold two publish records for this walk to be the one that was missing: \
         {:?}",
        rows.iter().map(|row| &row.action).collect::<Vec<_>>()
    );
    assert!(
        publishes[1] > publishes[0] + 1,
        "the successor's own `create` record sits between them: {publishes:?}"
    );

    // The walk. Every link, from the genesis seed to the head.
    let genesis = genesis_prev_hash(TENANT, plan_id().get());
    assert_eq!(
        rows[0].prev_hash.as_deref(),
        Some(genesis.as_slice()),
        "the first link is the segment's own seed, computed from the tenant and the chain rather \
         than read from the store"
    );
    for pair in rows.windows(2) {
        assert_eq!(
            pair[1].seq,
            pair[0].seq + 1,
            "the segment's positions are dense; a gap after {} is a link nobody can walk",
            pair[0].seq
        );
        assert_eq!(
            pair[1].prev_hash.as_deref(),
            Some(pair[0].row_hash.as_slice()),
            "row {} does not point at its predecessor's digest",
            pair[1].seq
        );
    }

    // A constant digest would satisfy every link above from row 1 on.
    let distinct: std::collections::BTreeSet<&[u8]> =
        rows.iter().map(|row| row.row_hash.as_slice()).collect();
    assert_eq!(
        distinct.len(),
        rows.len(),
        "two rows share a digest, so the adjacency above proves nothing"
    );
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
    //
    // The code is read off the **typed context** and compared by equality. This
    // assertion used to be `format!("{canonical:?}").contains("CONCURRENT_MUTATION")`,
    // which is the weak form this phase kept finding: a containment test over a
    // rendered document is satisfied by the code with a character **appended**, and
    // `WINDOW_OVERLAP` once passed as `WINDOW_OVERLAPX` under exactly that shape.
    // Over `Debug` of the whole error it is looser still, since any field that
    // happens to quote the reason satisfies it.
    let canonical = toolkit::api::canonical_prelude::CanonicalError::from(
        bss_pricing::infra::storage::repo_failure(&refusal),
    );
    assert_eq!(canonical.status_code(), 409, "{canonical:?}");
    match canonical {
        toolkit::api::canonical_prelude::CanonicalError::Aborted { ctx, .. } => {
            assert_eq!(ctx.reason, "CONCURRENT_MUTATION");
        }
        other => panic!("expected a 409 conflict, got {other:?}"),
    }
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
            PlanPublishUnit::plan_content(plan_id(), revision),
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
