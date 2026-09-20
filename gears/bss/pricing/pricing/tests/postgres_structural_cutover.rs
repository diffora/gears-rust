//! Shared structure across a logical line's markets, on Postgres.
//!
//! The engine twin of the two cases at the end of `sqlite_publish_commit.rs`.
//! What is Postgres-specific here is the **transaction**: the refusal below has
//! to take a real multi-statement commit back, across the outbox, the audit
//! chain, the read model, the version ref and both window writes, on the engine
//! that actually runs in production.
//!
//! The race the plan for this work asks for — a draft market edit against a
//! publication, under barriers — is `postgres_draft_windows.rs`'s
//! `a_draft_edit_is_serialized_against_publish`, which stages exactly that on
//! this engine through the same per-plan guard. It is not repeated here: two
//! tests racing one guard is two answers to one question, free to disagree.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;
mod pg_support;

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bss_pricing::config::LimitsConfig;
use bss_pricing::domain::approval::{DecisionBy, WithdrawAuthority};
use bss_pricing::domain::audit::AuditStamp;
use bss_pricing::domain::concurrency::RowVersion;
use bss_pricing::domain::contracts::{BillingAnchorPolicy, ProrationBasis, ProrationContract};
use bss_pricing::domain::draft_window::{DraftStart, DraftWindowAction, DraftWindowEntry};
use bss_pricing::domain::error::DomainError;
use bss_pricing::domain::instant::utc_ymd_hms;
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::money::{CurrencyCode, MinorAmount};
use bss_pricing::domain::plan::PlanShapePatch;
use bss_pricing::domain::plan_shape::{Frequency, PhaseKind, PlanPhase};
use bss_pricing::domain::price_record::PriceContent;
use bss_pricing::domain::price_row::{ModelKind, PriceRow};
use bss_pricing::domain::publish::{PlanPublishUnit, PublishAuthorization};
use bss_pricing::domain::scope_key::{
    ChargeKind, ChargeLineScopeKey, Cohort, MarketPriceScopeKey, PhaseId, PlanId, PriceEligibility,
    Region, SkuId,
};
use bss_pricing::domain::structural_schedule::STRUCTURE_CUTOVER_MISMATCH;
use bss_pricing::infra::approval::{ApprovalService, DecideRequest, RegionGrant};
use bss_pricing::infra::draft_window::{self, DraftWindowCommand};
use bss_pricing::infra::fixture_gate::FixtureGate;
use bss_pricing::infra::publish::PublishService;
use bss_pricing::infra::storage::entity::{charge_line_version, outbox, price};
use bss_pricing::infra::storage::repo::{
    NewPlanDraft, NewPriceDraft, PlanRepo, PlanShapeRepo, PriceRepo,
};
use bss_pricing_sdk::catalog_version::CatalogVersion;
use bss_pricing_sdk::catalog_version_registry::{CatalogVersionRegistryV1, PendingVersionRef};
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use time::OffsetDateTime;
use toolkit_canonical_errors::CanonicalError;
use toolkit_db::secure::{AccessScope, SecureEntityExt, SecureInsertExt, SecureUpdateExt};
use toolkit_db::{DBProvider, DbError};
use toolkit_security::SecurityContext;
use uuid::Uuid;

use pg_support::Pg;

const TENANT: Uuid = Uuid::from_u128(0x7e_11);
const ACTOR: Uuid = Uuid::from_u128(0xac_01);
const APPROVER: Uuid = Uuid::from_u128(0xac_02);
const PLAN: Uuid = Uuid::from_u128(0x91_a1);
const PHASE: Uuid = Uuid::from_u128(0x40_a5);
const OFFER_SKU: Uuid = Uuid::from_u128(0x5_c1);
const ROW_SKU: Uuid = Uuid::from_u128(5);
const TEST_CORRELATION: Uuid = Uuid::from_u128(0x_c0_11_a7_10);
const HOME_PRICE: Uuid = Uuid::from_u128(0xa0_01);
const SECOND_MARKET_PRICE: Uuid = Uuid::from_u128(0xa0_02);
const HOME_WINDOW: Uuid = Uuid::from_u128(0x_c0_7e);
const SECOND_MARKET_WINDOW: Uuid = Uuid::from_u128(0x_c0_7f);

fn now() -> OffsetDateTime {
    utc_ymd_hms(2099, 8, 3, 0, 0, 0)
}

fn stamp() -> AuditStamp {
    AuditStamp {
        actor_principal_id: ACTOR,
        recorded_at: now(),
        correlation_id: TEST_CORRELATION,
    }
}

fn scope() -> AccessScope {
    AccessScope::for_tenant(TENANT)
}

fn ctx() -> SecurityContext {
    SecurityContext::builder()
        .subject_id(ACTOR)
        .subject_tenant_id(TENANT)
        .build()
        .expect("a subject and a tenant are all a context needs")
}

fn committed_registry_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus/registry.toml")
}

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
    ) -> Result<PendingVersionRef, CanonicalError> {
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
    ) -> Result<Option<CatalogVersion>, CanonicalError> {
        Ok(None)
    }
}

/// One market of the plan's single logical line.
fn market(currency: &str, region: &str) -> MarketPriceScopeKey {
    MarketPriceScopeKey::new(
        ChargeLineScopeKey::new(
            PlanId::new(PLAN),
            PhaseId::new(PHASE),
            PriceEligibility::AllSubscriptions,
            ChargeKind::Recurring,
            Cohort::None,
            SkuId::new(ROW_SKU),
        )
        .expect("the class pairs with cohort none"),
        CurrencyCode::new(currency).expect("three letters"),
        Region::new(region).expect("a non-blank region"),
    )
}

fn publishable_row() -> PriceContent {
    let mut row = {
        let mut descriptor_row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
        descriptor_row.gl_code_ref = Some("4000".to_owned());
        descriptor_row
    };
    row.amount_minor = Some(MinorAmount::new(9_900).expect("a non-negative amount"));
    PriceContent {
        row,
        tax_inclusive: false,
        tax_category_ref: Some("standard".to_owned()),
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

struct Store {
    #[allow(
        dead_code,
        reason = "the container must outlive the connections it lends"
    )]
    pg: Pg,
    db: DBProvider<DbError>,
    plans: PlanRepo,
    prices: PriceRepo,
    publish: PublishService,
}

/// A plan the whole rule set passes, selling one market, with its covering
/// intention authored. Returns the store and the plan's current row version.
async fn seeded_plan() -> (Store, RowVersion) {
    let pg = Pg::applied().await;
    let db = DBProvider::<DbError>::new(pg.db().await);
    common::declare_fixture_regions(&db, TENANT).await;
    let plans = PlanRepo::new(db.clone());
    let shapes = PlanShapeRepo::new(db.clone());
    let prices = PriceRepo::new(db.clone());
    let publish = PublishService::new(
        db.clone(),
        &LimitsConfig::default(),
        FixtureGate::load(&committed_registry_path()),
        Arc::new(RegistryDouble::default()) as Arc<dyn CatalogVersionRegistryV1>,
    )
    .with_product_catalog(Arc::new(common::FixtureCatalog::default()))
    .resolve_skus(&ctx(), &common::FixtureCatalog::default().sku_ids())
    .await
    .expect("fixture registry");

    let created = plans
        .create_draft(
            &scope(),
            NewPlanDraft {
                plan_name: None,
                plan_id: PlanId::new(PLAN),
                tenant_id: TENANT,
                created_by: ACTOR,
                created_at_utc: now(),
                sku_id: OFFER_SKU,
                plan_tier: Some("gold".to_owned()),
                frequency: Some(Frequency::Monthly),
                plan_tier_override: false,
                purchase_min_qty: None,
                purchase_max_qty: None,
                descriptor_ext: std::collections::BTreeMap::new(),
                available_from: None,
                available_to: None,
                cloned_from: None,
                correlation_id: TEST_CORRELATION,
            },
        )
        .await
        .expect("create the draft plan");
    let after_phases = shapes
        .replace_phases(
            &scope(),
            TENANT,
            PlanId::new(PLAN),
            created.revision,
            created.row_version,
            vec![PlanPhase {
                phase_id: PhaseId::new(PHASE),
                kind: PhaseKind::Evergreen,
                display_name: None,
                ordinal: 0,
                converts_to_phase_id: None,
                phase_duration_days: None,
            }],
            stamp(),
        )
        .await
        .expect("attach the phase chain");
    let after_descriptors = plans
        .update_draft(
            &scope(),
            TENANT,
            PlanId::new(PLAN),
            created.revision,
            after_phases.row_version,
            PlanShapePatch {
                descriptor_ext: Some(std::collections::BTreeMap::new()),
                ..Default::default()
            },
            stamp(),
        )
        .await
        .expect("attach the descriptor set");

    let store = Store {
        pg,
        db,
        plans,
        prices,
        publish,
    };
    author_market(&store, HOME_PRICE, &market("EUR", "eu"), None).await;
    let version = author_window(
        &store,
        after_descriptors.row_version,
        HOME_PRICE,
        HOME_WINDOW,
    )
    .await;
    (store, version)
}

async fn author_market(
    store: &Store,
    price_id: Uuid,
    key: &MarketPriceScopeKey,
    line_version_id: Option<Uuid>,
) {
    store
        .prices
        .create_draft(
            &scope(),
            TENANT,
            NewPriceDraft {
                price_id,
                line_version_id,
                market_price_id: None,
                scope_key: key.clone(),
                content: publishable_row(),
                created_by: ACTOR,
                created_at_utc: now(),
                correlation_id: TEST_CORRELATION,
            },
        )
        .await
        .expect("author the market's price row");
}

async fn author_window(
    store: &Store,
    expected: RowVersion,
    price_id: Uuid,
    window_id: Uuid,
) -> RowVersion {
    let conn = store.db.conn().expect("conn");
    let next = draft_window::apply_command(
        &conn,
        &scope(),
        &bss_pricing::domain::draft_window::DraftWindowOwner {
            tenant_id: TENANT,
            plan_id: PLAN,
            plan_revision: 0,
        },
        expected.get(),
        DraftWindowCommand::Put(DraftWindowEntry {
            operation_id: window_id,
            action: DraftWindowAction::Create {
                window_id,
                price_id,
                start: DraftStart::AtPublish,
                effective_to: None,
            },
            reason_code: "launch".to_owned(),
        }),
        stamp(),
    )
    .await
    .expect("author the covering intention");
    RowVersion::new(next)
}

async fn commit(store: &Store, version: RowVersion) -> Result<(), DomainError> {
    store
        .publish
        .commit(
            &ctx(),
            &scope(),
            TENANT,
            PlanPublishUnit::plan_content(PlanId::new(PLAN), 0),
            version,
            PublishAuthorization::auto_publishable(),
            ACTOR,
            TEST_CORRELATION,
            now(),
        )
        .await
        .map(|_| ())
}

async fn line_versions(store: &Store) -> BTreeSet<Uuid> {
    let conn = store.db.conn().expect("conn");
    price::Entity::find()
        .secure()
        .scope_with(&scope())
        .filter(Condition::all().add(price::Column::TenantId.eq(TENANT)))
        .all(&conn)
        .await
        .expect("read the monetary rows")
        .into_iter()
        .map(|row| row.line_version_id)
        .collect()
}

/// The authoring door announces every seed act of its own (`PlanCreated`,
/// `PlanUpdated`, `PriceCreated`), so an unfiltered read makes "the commit wrote
/// nothing" false on a path that wrote nothing. `sqlite_publish_commit.rs`
/// carries the same exclusion and the argument for it.
const SEEDED_EVENT_NAMES: [&str; 3] = ["PlanCreated", "PlanUpdated", "PriceCreated"];

/// The events **this commit** wrote.
async fn commit_events(store: &Store) -> usize {
    let conn = store.db.conn().expect("conn");
    outbox::Entity::find()
        .secure()
        .scope_with(&scope())
        .filter(Condition::all().add(outbox::Column::TenantId.eq(TENANT)))
        .all(&conn)
        .await
        .expect("read the outbox")
        .into_iter()
        .filter(|row| !SEEDED_EVENT_NAMES.contains(&row.event_name.as_str()))
        .count()
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_two_market_revision_binds_both_markets_to_one_structure() {
    let (store, version) = seeded_plan().await;
    author_market(&store, SECOND_MARKET_PRICE, &market("USD", "us"), None).await;
    let version = author_window(&store, version, SECOND_MARKET_PRICE, SECOND_MARKET_WINDOW).await;

    commit(&store, version)
        .await
        .expect("a two-market revision publishes on Postgres");

    assert_eq!(
        line_versions(&store).await.len(),
        1,
        "one revision authors one structure, and both markets are priced against it"
    );
    let published = store
        .prices
        .list_for_plan(
            &scope(),
            TENANT,
            PlanId::new(PLAN),
            &[LifecycleState::Published],
        )
        .await
        .expect("read the published rows");
    assert_eq!(published.len(), 2, "both markets published");
    assert_eq!(commit_events(&store).await, 1, "one publication, one event");
}

/// A **second** charge-line version of the plan's one line, identical but for
/// its identity and revision.
///
/// `sqlite_publish_commit.rs`'s twin of this helper carries the argument: within
/// one revision the store derives one version per line, so the state the rule
/// guards is a cross-revision one and is reached directly rather than through a
/// supersession.
async fn clone_line_version(store: &Store) -> Uuid {
    let conn = store.db.conn().expect("conn");
    let original = charge_line_version::Entity::find()
        .secure()
        .scope_with(&scope())
        .filter(Condition::all().add(charge_line_version::Column::TenantId.eq(TENANT)))
        .one(&conn)
        .await
        .expect("read the line version")
        .expect("the seeded revision authored one");
    let clone_id = Uuid::from_u128(0x5e_c0_11);
    let mut clone: charge_line_version::ActiveModel = original.into();
    clone.line_version_id = sea_orm::ActiveValue::Set(clone_id);
    clone.plan_revision = sea_orm::ActiveValue::Set(1);
    clone.row_version = sea_orm::ActiveValue::Set(0);
    charge_line_version::Entity::insert(clone.clone())
        .secure()
        .scope_with_model(&scope(), &clone)
        .expect("scope the cloned version")
        .exec(&conn)
        .await
        .expect("insert the second version of one line");
    clone_id
}

/// Two markets of one line, covered over the same span, bound to two structure
/// versions. On Postgres the refusal has to unwind the whole commit: neither
/// market publishes, no event leaves, and the revision stays open.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn two_markets_on_two_structure_versions_roll_the_whole_commit_back() {
    let (store, version) = seeded_plan().await;
    let second_version = clone_line_version(&store).await;
    author_market(
        &store,
        SECOND_MARKET_PRICE,
        &market("USD", "us"),
        Some(second_version),
    )
    .await;
    let version = author_window(&store, version, SECOND_MARKET_PRICE, SECOND_MARKET_WINDOW).await;

    let refusal = commit(&store, version)
        .await
        .expect_err("two structures under one line cannot publish");
    match &refusal {
        DomainError::ValidationFailed(report) => {
            let codes: BTreeSet<&str> = report.violations.iter().map(|v| v.code.as_str()).collect();
            assert_eq!(
                codes,
                BTreeSet::from([STRUCTURE_CUTOVER_MISMATCH]),
                "exactly the fault this test staged: {report:?}"
            );
            assert_eq!(
                report.violations[0].subject,
                market("USD", "us").line().to_string()
            );
        }
        other => panic!("expected a validation report, got {other:?}"),
    }

    let published = store
        .prices
        .list_for_plan(
            &scope(),
            TENANT,
            PlanId::new(PLAN),
            &[LifecycleState::Published],
        )
        .await
        .expect("read the published rows");
    assert!(
        published.is_empty(),
        "neither market published: {published:?}"
    );
    assert_eq!(commit_events(&store).await, 0, "no event left the gear");
    let still = store
        .plans
        .find_revision(&scope(), TENANT, PlanId::new(PLAN), 0)
        .await
        .expect("read the revision")
        .expect("it is there");
    assert_eq!(still.lifecycle_state, LifecycleState::Draft);
    assert_eq!(still.row_version, version, "its tag did not move either");
}

/// **The publish-versus-edit window, for the one edit only the normalized pin
/// sees, on the engine that runs in production.**
///
/// A unit is approved; the row's money is then re-bound to a twin structure
/// version of identical content; the commit presents the approved digest. No
/// resolved row, no row version and no revision version has moved, so nothing
/// but the pin re-derived **inside the commit's transaction** can refuse — and
/// it does, leaving the revision open and nothing published. No mixed content
/// commits under the old approval.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_market_rebound_after_the_approve_cannot_commit_under_the_old_pin() {
    let (store, version) = seeded_plan().await;

    let approvals = ApprovalService::new(store.db.clone());
    let approval_id = Uuid::from_u128(0xa9_21);
    approvals
        .submit(
            &scope(),
            TENANT,
            PlanId::new(PLAN),
            approval_id,
            serde_json::json!({ "material": true, "reason": "noConfiguredThreshold" }),
            stamp(),
        )
        .await
        .expect("open the pending unit");
    let record = approvals
        .decide(
            &scope(),
            TENANT,
            DecideRequest {
                approval_id,
                decision: DecisionBy::Approve(APPROVER),
                reason: None,
                approver_regions: RegionGrant::Explicit(BTreeSet::from([
                    Region::new("eu").expect("a non-blank region")
                ])),
                stamp: AuditStamp {
                    actor_principal_id: APPROVER,
                    recorded_at: now(),
                    correlation_id: TEST_CORRELATION,
                },
                withdraw_authority: WithdrawAuthority::OwnUnitsOnly,
            },
        )
        .await
        .expect("an independent principal approves it");

    let twin = clone_line_version(&store).await;
    let conn = store.db.conn().expect("conn");
    let moved = price::Entity::update_many()
        .secure()
        .scope_with(&scope())
        .col_expr(
            price::Column::LineVersionId,
            sea_orm::sea_query::Expr::value(twin),
        )
        .filter(Condition::all().add(price::Column::PriceId.eq(HOME_PRICE)))
        .exec(&conn)
        .await
        .expect("re-bind the money");
    assert_eq!(moved.rows_affected, 1);

    let refused = store
        .publish
        .commit(
            &ctx(),
            &scope(),
            TENANT,
            PlanPublishUnit::plan_content(PlanId::new(PLAN), 0),
            version,
            PublishAuthorization::approved(
                record.approval_id,
                record.submitter_principal,
                record.approver_principal.expect("an approved record"),
                record
                    .content_hash
                    .as_slice()
                    .try_into()
                    .expect("a 32-byte digest"),
            ),
            ACTOR,
            TEST_CORRELATION,
            now(),
        )
        .await
        .expect_err("the structure approved is not the structure this would freeze");
    assert!(
        matches!(refused, DomainError::ApprovalContentMismatch(_)),
        "got {refused:?}"
    );

    let still = store
        .plans
        .find_revision(&scope(), TENANT, PlanId::new(PLAN), 0)
        .await
        .expect("read the revision")
        .expect("it is there");
    assert_eq!(still.lifecycle_state, LifecycleState::Draft);
    assert_eq!(commit_events(&store).await, 0, "no event left the gear");
}
