//! Revision-owned draft windows, captured baselines and the per-plan guard —
//! repository and schema, on Postgres.
//!
//! Mirrors `sqlite_draft_windows.rs`. Overlap is judged against the composed
//! proposal, not across independent revisions.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;
mod pg_support;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bss_pricing::config::LimitsConfig;
use bss_pricing::domain::approval::{DecisionBy, WithdrawAuthority};
use bss_pricing::domain::audit::AuditStamp;
use bss_pricing::domain::concurrency::RowVersion;
use bss_pricing::domain::contracts::{BillingAnchorPolicy, ProrationBasis, ProrationContract};
use bss_pricing::domain::draft_window::{
    DraftStart, DraftWindowAction, DraftWindowEntry, DraftWindowOwner, WindowBaseline,
};
use bss_pricing::domain::error::DomainError;
use bss_pricing::domain::instant::utc_ymd_hms;
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::materiality::{ThresholdBasis, ThresholdEntry};
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
use bss_pricing::infra::approval::{ApprovalService, DecideRequest, RegionGrant};
use bss_pricing::infra::draft_window::{self, DraftWindowCommand};
use bss_pricing::infra::fixture_gate::FixtureGate;
use bss_pricing::infra::publish::PublishService;
use bss_pricing::infra::storage::RepoError;
use bss_pricing::infra::storage::entity::{plan, price};
use bss_pricing::infra::storage::repo::draft_window_repo;
use bss_pricing::infra::storage::repo::price_repo;
use bss_pricing::infra::storage::repo::window_baseline_repo;
use bss_pricing::infra::storage::repo::window_guard_repo;
use bss_pricing::infra::storage::repo::window_repo;
use bss_pricing::infra::storage::repo::{
    NewPlanDraft, NewPriceDraft, PlanRepo, PlanShapeRepo, PriceRepo,
};
use bss_pricing::infra::threshold::{AssertedPolicy, ThresholdService};
use bss_pricing::infra::window::{WindowMutationOutcome, WindowService};
use bss_pricing_sdk::catalog_version::CatalogVersion;
use bss_pricing_sdk::catalog_version_registry::{CatalogVersionRegistryV1, PendingVersionRef};
use serde_json::json;
use tokio::sync::Notify;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;

use pg_support::Pg;
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, EntityTrait, Statement,
};
use time::OffsetDateTime;
use toolkit_db::secure::{AccessScope, SecureInsertExt, SecureUpdateExt, TxError};
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

const TENANT: Uuid = Uuid::from_u128(0x7e_11);
const OTHER_TENANT: Uuid = Uuid::from_u128(0x7e_22);
const ACTOR: Uuid = Uuid::from_u128(0xac_01);
const PLAN: Uuid = Uuid::from_u128(0x91_a1);
const OTHER_PLAN: Uuid = Uuid::from_u128(0x91_a2);
const PHASE: Uuid = Uuid::from_u128(0x40_a5);
const SKU: Uuid = Uuid::from_u128(5);
const ROW: Uuid = Uuid::from_u128(0xa0_01);
const FOREIGN_ROW: Uuid = Uuid::from_u128(0xa0_99);
const TEST_CORRELATION: Uuid = Uuid::from_u128(0x_c0_11_a7_10);
const APPROVER: Uuid = Uuid::from_u128(0xac_02);
const OFFER_SKU: Uuid = Uuid::from_u128(0x5_c1);
const COVER_WINDOW: Uuid = Uuid::from_u128(0x_c0_7e);
/// Successor covering after the materialized first-publish window is closed at
/// [`coverage_to`], so Working still satisfies `inst-wc-required`.
const TAIL_WINDOW: Uuid = Uuid::from_u128(0x_c0_7f);
const DRAFT_A: Uuid = Uuid::from_u128(0x_d7_a1);
const DRAFT_B: Uuid = Uuid::from_u128(0x_d7_b2);
const LIVE_A: Uuid = Uuid::from_u128(0x_e1);
const LIVE_B: Uuid = Uuid::from_u128(0x_e2);

fn t(day: u32) -> OffsetDateTime {
    utc_ymd_hms(2099, 9, day, 0, 0, 0)
}

fn stamp() -> AuditStamp {
    AuditStamp {
        actor_principal_id: ACTOR,
        recorded_at: t(1),
        correlation_id: TEST_CORRELATION,
    }
}

fn scope() -> AccessScope {
    AccessScope::for_tenant(TENANT)
}

fn owner() -> DraftWindowOwner {
    DraftWindowOwner {
        tenant_id: TENANT,
        plan_id: PLAN,
        plan_revision: 0,
    }
}

fn new_draft(plan_id: PlanId, tenant_id: Uuid) -> NewPlanDraft {
    NewPlanDraft {
        plan_name: "Fixture Plan".to_owned(),
        plan_id,
        tenant_id,
        created_by: ACTOR,
        created_at_utc: t(1),
        sku_id: SKU,
        plan_tier: None,
        frequency: None,
        purchase_min_qty: None,
        purchase_max_qty: None,
        descriptor_ext: std::collections::BTreeMap::new(),
        available_from: None,
        available_to: None,
        cloned_from: None,
        correlation_id: TEST_CORRELATION,
    }
}

fn symbolic_create(window_id: Uuid) -> DraftWindowEntry {
    DraftWindowEntry {
        operation_id: window_id,
        action: DraftWindowAction::Create {
            window_id,
            price_id: ROW,
            start: DraftStart::AtPublish,
            effective_to: None,
        },
        reason_code: "launch".to_owned(),
    }
}

struct Store {
    pg: Pg,
    db: DBProvider<DbError>,
    plans: PlanRepo,
    shapes: PlanShapeRepo,
    prices: PriceRepo,
    raw: DatabaseConnection,
}

async fn store() -> Store {
    let pg = Pg::applied().await;
    let db = DBProvider::<DbError>::new(pg.db().await);
    let plans = PlanRepo::new(db.clone());
    let shapes = PlanShapeRepo::new(db.clone());
    let prices = PriceRepo::new(db.clone());
    let raw = pg.raw().await;
    Store {
        pg,
        db,
        plans,
        shapes,
        prices,
        raw,
    }
}

async fn seed_price(store: &Store, tenant_id: Uuid, plan_id: Uuid, price_id: Uuid) {
    let conn = store.db.conn().expect("scoped connection");
    let row_scope = AccessScope::for_tenant(tenant_id);
    let seeded_graph = common::seed_charge_graph(
        &conn,
        &row_scope,
        &common::ChargeGraphSeed {
            tenant_id,
            plan_id,
            phase: PHASE,
            sku_id: SKU,
            charge_kind: "recurring".to_owned(),
            currency: "USD".to_owned(),
            region: "EU".to_owned(),
            lifecycle_state: "draft".to_owned(),
            model_kind: Some("flat".to_owned()),
            created_by: ACTOR,
            created_at_utc: t(1),
            ..Default::default()
        },
    )
    .await;
    let row = price::ActiveModel {
        plan_revision: Set(1),
        charge_line_id: Set(seeded_graph.charge_line_id),
        line_version_id: Set(seeded_graph.line_version_id),
        market_price_id: Set(seeded_graph.market_price_id),
        price_id: Set(price_id),
        tenant_id: Set(tenant_id),
        plan_id: Set(plan_id),
        amount_minor: Set(Some(1_000)),
        lifecycle_state: Set("draft".to_owned()),
        created_by: Set(ACTOR),
        created_at_utc: Set(t(1)),
        ..price::ActiveModel::default()
    };
    price::Entity::insert(row.clone())
        .secure()
        .scope_with_model(&row_scope, &row)
        .expect("scope the seeded price row")
        .exec(&conn)
        .await
        .unwrap_or_else(|e| panic!("seed price row {price_id}: {e}"));
}

async fn seeded_plan() -> Store {
    let store = store().await;
    store
        .plans
        .create_draft(&scope(), new_draft(PlanId::new(PLAN), TENANT))
        .await
        .expect("create revision 0");
    seed_price(&store, TENANT, PLAN, ROW).await;
    store
}

async fn flip_state(store: &Store, state: LifecycleState) {
    let conn = store.db.conn().expect("conn");
    let result = plan::Entity::update_many()
        .secure()
        .scope_with(&scope())
        .col_expr(plan::Column::LifecycleState, Expr::value(state.as_str()))
        .filter(
            Condition::all()
                .add(plan::Column::PlanId.eq(PLAN))
                .add(plan::Column::Revision.eq(0_i64)),
        )
        .exec(&conn)
        .await
        .expect("flip the lifecycle state");
    assert_eq!(result.rows_affected, 1, "the seed must have moved one row");
}

async fn exec(conn: &DatabaseConnection, sql: &str) -> Result<(), sea_orm::DbErr> {
    conn.execute_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        sql.to_owned(),
    ))
    .await
    .map(|_| ())
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_symbolic_create_round_trips_with_a_null_start() {
    let store = seeded_plan().await;
    let conn = store.db.conn().expect("conn");
    let window_id = Uuid::from_u128(0x_d7_01);
    draft_window_repo::put(&conn, &scope(), &owner(), &symbolic_create(window_id))
        .await
        .expect("put a symbolic create");
    let listed = draft_window_repo::list(&conn, &scope(), &owner())
        .await
        .expect("list");
    assert_eq!(listed, vec![symbolic_create(window_id)]);
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_missing_owner_is_not_found() {
    let store = seeded_plan().await;
    let conn = store.db.conn().expect("conn");
    let missing = DraftWindowOwner {
        tenant_id: TENANT,
        plan_id: PLAN,
        plan_revision: 9,
    };
    let err = draft_window_repo::put(
        &conn,
        &scope(),
        &missing,
        &symbolic_create(Uuid::from_u128(0x_d7_03)),
    )
    .await
    .expect_err("no such revision");
    assert!(
        matches!(err, RepoError::NotFound { .. }),
        "missing owner must be NotFound, got: {err:?}"
    );
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_foreign_tenant_cannot_see_the_owner() {
    let store = seeded_plan().await;
    let conn = store.db.conn().expect("conn");
    let err = draft_window_repo::put(
        &conn,
        &AccessScope::for_tenant(OTHER_TENANT),
        &owner(),
        &symbolic_create(Uuid::from_u128(0x_d7_04)),
    )
    .await
    .expect_err("foreign tenant");
    assert!(
        matches!(err, RepoError::NotFound { .. }),
        "foreign tenant must be indistinguishable from absent, got: {err:?}"
    );
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_price_on_another_plan_is_refused() {
    let store = seeded_plan().await;
    store
        .plans
        .create_draft(&scope(), new_draft(PlanId::new(OTHER_PLAN), TENANT))
        .await
        .expect("second plan");
    seed_price(&store, TENANT, OTHER_PLAN, FOREIGN_ROW).await;
    let conn = store.db.conn().expect("conn");
    let window_id = Uuid::from_u128(0x_d7_05);
    let entry = DraftWindowEntry {
        operation_id: window_id,
        action: DraftWindowAction::Create {
            window_id,
            price_id: FOREIGN_ROW,
            start: DraftStart::AtPublish,
            effective_to: None,
        },
        reason_code: "wrong parent".to_owned(),
    };
    let err = draft_window_repo::put(&conn, &scope(), &owner(), &entry)
        .await
        .expect_err("price belongs to another plan");
    assert!(
        matches!(err, RepoError::NotFound { .. }),
        "foreign plan price must be NotFound, got: {err:?}"
    );
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_second_operation_on_the_same_target_is_refused() {
    let store = seeded_plan().await;
    let conn = store.db.conn().expect("conn");
    let window_id = Uuid::from_u128(0x_d7_06);
    draft_window_repo::put(&conn, &scope(), &owner(), &symbolic_create(window_id))
        .await
        .expect("first create");
    let adjust = DraftWindowEntry {
        operation_id: Uuid::from_u128(0xd716),
        action: DraftWindowAction::AdjustEnd {
            window_id,
            effective_to: Some(t(12)),
        },
        reason_code: "duplicate target".to_owned(),
    };
    let err = draft_window_repo::put(&conn, &scope(), &owner(), &adjust)
        .await
        .expect_err("unique target");
    assert!(
        err.to_string().contains("uq_pricing_draft_window_target")
            || err.to_string().contains("draft window")
            || matches!(err, RepoError::ConcurrentMutation { .. }),
        "duplicate target must name the unique constraint, got: {err:?}"
    );
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_published_owner_rejects_put_and_keeps_the_row() {
    let store = seeded_plan().await;
    let conn = store.db.conn().expect("conn");
    let window_id = Uuid::from_u128(0x_d7_07);
    draft_window_repo::put(&conn, &scope(), &owner(), &symbolic_create(window_id))
        .await
        .expect("put while draft");
    flip_state(&store, LifecycleState::Published).await;
    let err = draft_window_repo::put(
        &conn,
        &scope(),
        &owner(),
        &symbolic_create(Uuid::from_u128(0x_d7_17)),
    )
    .await
    .expect_err("published owner is frozen");
    assert!(
        matches!(err, RepoError::NotDraft { .. }),
        "published owner must be NotDraft, got: {err:?}"
    );
    let listed = draft_window_repo::list(&conn, &scope(), &owner())
        .await
        .expect("list still works");
    assert_eq!(listed.len(), 1);
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn abandoning_a_revision_retains_its_intentions() {
    let store = seeded_plan().await;
    let conn = store.db.conn().expect("conn");
    let window_id = Uuid::from_u128(0x_d7_08);
    draft_window_repo::put(&conn, &scope(), &owner(), &symbolic_create(window_id))
        .await
        .expect("put while draft");
    store
        .plans
        .abandon_draft(
            &scope(),
            TENANT,
            PlanId::new(PLAN),
            0,
            RowVersion::new(0),
            stamp(),
        )
        .await
        .expect("abandon");
    let listed = draft_window_repo::list(&conn, &scope(), &owner())
        .await
        .expect("abandoned rows stay");
    assert_eq!(listed, vec![symbolic_create(window_id)]);
    let err = draft_window_repo::remove(&conn, &scope(), &owner(), window_id)
        .await
        .expect_err("abandoned owner is frozen");
    assert!(
        matches!(err, RepoError::NotDraft { .. }),
        "abandoned owner must be NotDraft, got: {err:?}"
    );
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_baseline_replace_round_trips() {
    let store = seeded_plan().await;
    let conn = store.db.conn().expect("conn");
    let captured = WindowBaseline {
        window_id: Uuid::from_u128(0x_b1_01),
        price_id: ROW,
        mutation_seq: 3,
        effective_from: t(2),
        effective_to: Some(t(9)),
        cancelled: false,
    };
    window_baseline_repo::replace(&conn, &scope(), &owner(), std::slice::from_ref(&captured))
        .await
        .expect("replace");
    let listed = window_baseline_repo::list(&conn, &scope(), &owner())
        .await
        .expect("list");
    assert_eq!(listed, vec![captured]);
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn plan_create_inserts_a_guard_row_that_acquire_can_lock() {
    let store = seeded_plan().await;
    let conn = store.db.conn().expect("conn");
    window_guard_repo::acquire(&conn, &scope(), TENANT, PLAN)
        .await
        .expect("first acquire");
    window_guard_repo::acquire(&conn, &scope(), TENANT, PLAN)
        .await
        .expect("second acquire");
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn acquire_without_a_guard_row_is_an_error() {
    let store = store().await;
    let conn = store.db.conn().expect("conn");
    let err = window_guard_repo::acquire(&conn, &scope(), TENANT, PLAN)
        .await
        .expect_err("no guard row");
    assert!(
        matches!(err, RepoError::NotFound { .. }),
        "zero affected rows must be NotFound, got: {err:?}"
    );
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_successor_revision_does_not_fail_the_guard_primary_key() {
    let store = seeded_plan().await;
    flip_state(&store, LifecycleState::Published).await;
    let opened = store
        .plans
        .open_revision(&scope(), TENANT, PlanId::new(PLAN), stamp())
        .await
        .expect("successor must not collide on the per-plan guard");
    assert_eq!(opened.revision, 1);
    let conn = store.db.conn().expect("conn");
    window_guard_repo::acquire(&conn, &scope(), TENANT, PLAN)
        .await
        .expect("the one guard row is still acquirable");
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn at_publish_cannot_carry_an_authored_start() {
    let store = seeded_plan().await;
    let err = exec(
        &store.raw,
        &format!(
            "INSERT INTO bss.pricing_draft_window (
                tenant_id, plan_id, plan_revision, operation_id, target_window_id,
                action, price_id, start_kind, effective_from, effective_to, reason_code)
             VALUES ('{TENANT}', '{PLAN}', 0, '{ROW}', '{ROW}',
                'create', '{ROW}', 'at_publish', '2099-09-08 00:00:00+00', NULL, 'launch')"
        ),
    )
    .await
    .expect_err("at_publish plus an instant is incoherent");
    assert!(
        err.to_string().contains("chk_pricing_draft_window_shape"),
        "must be the shape CHECK, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Guard serialization: T1 is a named door parked after its write; T2 is the
// other named door. Copy `postgres_window.rs`'s choreography.
// ---------------------------------------------------------------------------

const RACE_TIMEOUT: Duration = Duration::from_secs(30);

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

fn race_ctx() -> SecurityContext {
    SecurityContext::builder()
        .subject_id(ACTOR)
        .subject_tenant_id(TENANT)
        .build()
        .expect("a subject and a tenant are all a context needs")
}

fn race_now() -> OffsetDateTime {
    utc_ymd_hms(2099, 8, 3, 0, 0, 0)
}

fn race_stamp() -> AuditStamp {
    AuditStamp {
        actor_principal_id: ACTOR,
        recorded_at: race_now(),
        correlation_id: TEST_CORRELATION,
    }
}

fn stamp_of(actor: Uuid, at: OffsetDateTime) -> AuditStamp {
    AuditStamp {
        actor_principal_id: actor,
        recorded_at: at,
        correlation_id: TEST_CORRELATION,
    }
}

fn coverage_from() -> OffsetDateTime {
    let (y, m, d) = common::COVERAGE_FROM_UTC;
    utc_ymd_hms(y, m, d, 0, 0, 0)
}

fn coverage_to() -> OffsetDateTime {
    let (y, m, d) = common::COVERAGE_TO_UTC;
    utc_ymd_hms(y, m, d, 0, 0, 0)
}

fn owner_rev(plan_revision: u64) -> DraftWindowOwner {
    DraftWindowOwner {
        tenant_id: TENANT,
        plan_id: PLAN,
        plan_revision,
    }
}

fn covering_create(
    window_id: Uuid,
    reason: &str,
    start: OffsetDateTime,
    effective_to: Option<OffsetDateTime>,
) -> DraftWindowEntry {
    DraftWindowEntry {
        operation_id: window_id,
        action: DraftWindowAction::Create {
            window_id,
            price_id: ROW,
            start: DraftStart::At(start),
            effective_to,
        },
        reason_code: reason.to_owned(),
    }
}

fn covering_at_publish(window_id: Uuid, reason: &str) -> DraftWindowEntry {
    DraftWindowEntry {
        operation_id: window_id,
        action: DraftWindowAction::Create {
            window_id,
            price_id: ROW,
            start: DraftStart::AtPublish,
            effective_to: None,
        },
        reason_code: reason.to_owned(),
    }
}

fn publishable_row(amount_minor: i64) -> PriceContent {
    let mut row = {
        let mut descriptor_row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
        descriptor_row.gl_code_ref = Some("4000".to_owned());
        descriptor_row
    };
    row.amount_minor = Some(MinorAmount::new(amount_minor).expect("a non-negative amount"));
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

fn publishable_scope_key() -> MarketPriceScopeKey {
    MarketPriceScopeKey::new(
        ChargeLineScopeKey::new(
            PlanId::new(PLAN),
            PhaseId::new(PHASE),
            PriceEligibility::AllSubscriptions,
            ChargeKind::Recurring,
            Cohort::None,
            SkuId::new(Uuid::from_u128(5)),
        )
        .expect("scope key"),
        CurrencyCode::new("EUR").expect("currency"),
        Region::new("eu").expect("region"),
    )
}

fn committed_registry_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus/registry.toml")
}

fn is_version_overlap_or_baseline(err: &DomainError) -> bool {
    matches!(
        err,
        DomainError::StaleVersion(_)
            | DomainError::WindowOverlap(_)
            | DomainError::WindowBaselineChanged(_)
    )
}

fn domain_from_tx<T: std::fmt::Debug>(
    out: Result<T, TxError<DomainError>>,
) -> Result<T, DomainError> {
    match out {
        Ok(value) => Ok(value),
        Err(TxError::Domain(err)) => Err(err),
        Err(other) => panic!("transaction failed outside the domain: {other:?}"),
    }
}

fn must_commit(outcome: &WindowMutationOutcome) {
    match outcome {
        WindowMutationOutcome::Committed(_) => {}
        WindowMutationOutcome::SubmittedForApproval(_) => {
            panic!(
                "named live door submitted for approval instead of writing; install the EUR threshold"
            )
        }
    }
}

async fn publish_service(db: DBProvider<DbError>, registry: Arc<RegistryDouble>) -> PublishService {
    PublishService::new(
        db,
        &LimitsConfig::default(),
        FixtureGate::load(&committed_registry_path()),
        registry as Arc<dyn CatalogVersionRegistryV1>,
    )
    .with_product_catalog(Arc::new(common::FixtureCatalog::default()))
    .resolve_skus(&race_ctx(), &common::FixtureCatalog::default().sku_ids())
    .await
    .expect("fixture registry")
}

fn windows_on(db: DBProvider<DbError>, registry: Arc<RegistryDouble>) -> WindowService {
    WindowService::new(db, registry as Arc<dyn CatalogVersionRegistryV1>)
}

async fn seed_publishable_draft() -> (Store, u64) {
    let store = store().await;
    common::declare_fixture_regions(&store.db, TENANT).await;
    let created = store
        .plans
        .create_draft(
            &scope(),
            NewPlanDraft {
                plan_name: "Fixture Plan".to_owned(),
                plan_id: PlanId::new(PLAN),
                tenant_id: TENANT,
                created_by: ACTOR,
                created_at_utc: race_now(),
                sku_id: OFFER_SKU,
                plan_tier: Some("gold".to_owned()),
                frequency: Some(Frequency::Monthly),
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
    let after_phases = store
        .shapes
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
            race_stamp(),
        )
        .await
        .expect("attach the phase chain");
    let after_descriptors = store
        .plans
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
            race_stamp(),
        )
        .await
        .expect("attach the descriptor set");
    store
        .prices
        .create_draft(
            &scope(),
            TENANT,
            NewPriceDraft {
                price_id: ROW,
                line_version_id: None,
                market_price_id: None,
                scope_key: publishable_scope_key(),
                content: publishable_row(9_900),
                created_by: ACTOR,
                created_at_utc: race_now(),
                correlation_id: TEST_CORRELATION,
            },
        )
        .await
        .expect("author the price row");
    (store, after_descriptors.row_version.get())
}

async fn apply_on_store(
    store: &Store,
    revision: u64,
    expected: u64,
    command: DraftWindowCommand,
) -> u64 {
    let conn = store.db.conn().expect("conn");
    draft_window::apply_command(
        &conn,
        &scope(),
        &owner_rev(revision),
        expected,
        command,
        race_stamp(),
    )
    .await
    .expect("apply draft-window command")
}

async fn capture_empty_baseline(store: &Store, expected: u64) -> u64 {
    apply_on_store(store, 0, expected, DraftWindowCommand::RefreshBaseline).await
}

async fn put_covering(store: &Store, expected: u64, reason: &str) -> u64 {
    apply_on_store(
        store,
        0,
        expected,
        DraftWindowCommand::Put(covering_create(COVER_WINDOW, reason, coverage_from(), None)),
    )
    .await
}

async fn first_publish(store: &Store, registry: Arc<RegistryDouble>, expected: u64) {
    let publish = publish_service(store.db.clone(), registry).await;
    publish
        .commit(
            &race_ctx(),
            &scope(),
            TENANT,
            PlanPublishUnit::plan_content(PlanId::new(PLAN), 0),
            RowVersion::new(expected),
            PublishAuthorization::auto_publishable(),
            ACTOR,
            TEST_CORRELATION,
            race_now(),
        )
        .await
        .expect("first publish of the live-schedulable fixture");
}

/// Bound the open-ended window `first_publish` materializes so a later live
/// schedule can occupy the tail. `WindowService` refuses that bound
/// (`WINDOW_TRAILING_VOID`: schedule the successor first); this race is about
/// the guard, not that rule, so the store door plants the finite covering.
async fn close_materialized_covering_at_to(store: &Store) {
    let seq = live_seq(store, COVER_WINDOW).await;
    let conn = store.db.conn().expect("conn");
    window_repo::adjust_effective_to(
        &conn,
        &scope(),
        TENANT,
        COVER_WINDOW,
        Some(coverage_to()),
        seq,
        race_stamp(),
    )
    .await
    .expect("fixture: bound the materialized covering so the race can occupy the tail");
}

async fn install_eur_threshold(store: &Store) {
    let thresholds = ThresholdService::new(store.db.clone());
    let unit = Uuid::from_u128(0x_aa_70);
    let asserted = AssertedPolicy {
        tag: thresholds
            .state(&scope(), TENANT)
            .await
            .expect("the policy state reads")
            .tag(),
        now: race_now(),
    };
    thresholds
        .propose(
            &scope(),
            TENANT,
            unit,
            race_now(),
            vec![ThresholdEntry {
                currency: CurrencyCode::new("EUR").expect("a valid code"),
                basis: ThresholdBasis::Absolute { minor: 500 },
            }],
            bss_pricing::domain::materiality::DEFAULT_APPROVER_COUNT,
            asserted,
            json!({ "material": true, "reason": "alwaysMaterialTrigger" }),
            race_stamp(),
        )
        .await
        .expect("the proposal opens its unit");
    ApprovalService::new(store.db.clone())
        .decide(
            &scope(),
            TENANT,
            DecideRequest {
                approval_id: unit,
                decision: DecisionBy::Approve(APPROVER),
                reason: None,
                approver_regions: RegionGrant::Explicit(std::collections::BTreeSet::from([
                    Region::new("eu").expect("a non-blank region"),
                ])),
                stamp: stamp_of(APPROVER, utc_ymd_hms(2099, 8, 3, 1, 0, 0)),
                withdraw_authority: WithdrawAuthority::OwnUnitsOnly,
            },
        )
        .await
        .expect("an independent principal puts the policy in force");
}

async fn price_row_version(store: &Store) -> i64 {
    store
        .prices
        .find(&scope(), TENANT, ROW)
        .await
        .expect("read the price")
        .expect("the row exists")
        .row_version
        .get()
        .try_into()
        .expect("row_version fits i64")
}

async fn live_seq(store: &Store, window_id: Uuid) -> u64 {
    let conn = store.db.conn().expect("conn");
    window_repo::find(&conn, &scope(), TENANT, window_id)
        .await
        .expect("read the window")
        .expect("the window exists")
        .mutation_seq
}

async fn count_sql(conn: &DatabaseConnection, sql: &str) -> i64 {
    conn.query_one_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        sql.to_owned(),
    ))
    .await
    .expect("count query")
    .expect("one row")
    .try_get::<i64>("", "n")
    .expect("n")
}

async fn committed_window_ids(conn: &DatabaseConnection) -> Vec<String> {
    conn.query_all_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!(
            "SELECT w.window_id::text AS id
               FROM bss.pricing_price_window w
               JOIN bss.pricing_price p ON p.price_id = w.price_id
              WHERE p.plan_id = '{PLAN}' AND w.state <> 'cancelled'
              ORDER BY w.window_id"
        ),
    ))
    .await
    .expect("list windows")
    .into_iter()
    .map(|row| row.try_get::<String>("", "id").expect("id"))
    .collect()
}

async fn overlapping_non_cancelled(conn: &DatabaseConnection) -> i64 {
    count_sql(
        conn,
        &format!(
            "SELECT count(*)::bigint AS n
               FROM bss.pricing_price_window a
               JOIN bss.pricing_price pa ON pa.price_id = a.price_id
               JOIN bss.pricing_price_window b
                 ON a.price_id = b.price_id
                AND a.window_id < b.window_id
                AND a.state <> 'cancelled'
                AND b.state <> 'cancelled'
                AND a.effective_from < COALESCE(b.effective_to, 'infinity'::timestamptz)
                AND b.effective_from < COALESCE(a.effective_to, 'infinity'::timestamptz)
              WHERE pa.plan_id = '{PLAN}'"
        ),
    )
    .await
}

async fn artifact_counts(conn: &DatabaseConnection) -> (i64, i64, i64, i64) {
    let windows = count_sql(
        conn,
        &format!(
            "SELECT count(*)::bigint AS n
               FROM bss.pricing_price_window w
               JOIN bss.pricing_price p ON p.price_id = w.price_id
              WHERE p.plan_id = '{PLAN}'"
        ),
    )
    .await;
    let audit = count_sql(
        conn,
        &format!(
            "SELECT count(*)::bigint AS n FROM bss.pricing_audit_log WHERE tenant_id = '{TENANT}'"
        ),
    )
    .await;
    let outbox = count_sql(
        conn,
        &format!(
            "SELECT count(*)::bigint AS n FROM bss.pricing_outbox WHERE tenant_id = '{TENANT}'"
        ),
    )
    .await;
    let versions = count_sql(
        conn,
        &format!(
            "SELECT count(*)::bigint AS n FROM bss.pricing_catalog_version_ref WHERE tenant_id = '{TENANT}'"
        ),
    )
    .await;
    (windows, audit, outbox, versions)
}

/// published window IDs == IDs in the authorized operation set
/// published row versions == versions in the authorized candidate set
/// no committed key has overlapping non-cancelled intervals
/// failed transaction adds no window, audit success, outbox event or version reference
async fn assert_invariants(
    conn: &DatabaseConnection,
    authorized_window_ids: &[String],
    authorized_row_version: i64,
    before_artifacts: (i64, i64, i64, i64),
    failed_txn_added_nothing: bool,
) {
    let windows = committed_window_ids(conn).await;
    assert_eq!(
        windows, authorized_window_ids,
        "published window IDs == IDs in the authorized operation set"
    );
    let versions = count_sql(
        conn,
        &format!(
            "SELECT count(*)::bigint AS n FROM bss.pricing_price \
             WHERE plan_id = '{PLAN}' AND row_version <> {authorized_row_version}"
        ),
    )
    .await;
    assert_eq!(
        versions, 0,
        "published row versions == versions in the authorized candidate set"
    );
    assert_eq!(
        overlapping_non_cancelled(conn).await,
        0,
        "no committed key has overlapping non-cancelled intervals"
    );
    if failed_txn_added_nothing {
        assert_eq!(
            artifact_counts(conn).await,
            before_artifacts,
            "failed transaction adds no window, audit success, outbox event or version reference"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires Docker (testcontainers)"]
async fn a_draft_edit_is_serialized_against_publish() {
    let (store, version) = seed_publishable_draft().await;
    let version = capture_empty_baseline(&store, version).await;
    let version = put_covering(&store, version, "cover").await;
    let registry = Arc::new(RegistryDouble::default());
    let t2 = publish_service(
        DBProvider::<DbError>::new(store.pg.db().await),
        Arc::clone(&registry),
    )
    .await;

    let wrote = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let first = {
        let db = store.pg.db().await;
        let (wrote, release) = (Arc::clone(&wrote), Arc::clone(&release));
        tokio::spawn(async move {
            let (_db, out) = db
                .in_transaction::<u64, DomainError, _>(move |txn| {
                    Box::pin(async move {
                        let next = draft_window::apply_command(
                            txn,
                            &scope(),
                            &owner_rev(0),
                            version,
                            DraftWindowCommand::Put(covering_create(
                                COVER_WINDOW,
                                "raceReplace",
                                coverage_from(),
                                None,
                            )),
                            race_stamp(),
                        )
                        .await?;
                        wrote.notify_one();
                        release.notified().await;
                        Ok(next)
                    })
                })
                .await;
            out
        })
    };
    wrote.notified().await;

    let second = tokio::spawn(async move {
        t2.commit(
            &race_ctx(),
            &scope(),
            TENANT,
            PlanPublishUnit::plan_content(PlanId::new(PLAN), 0),
            RowVersion::new(version),
            PublishAuthorization::auto_publishable(),
            ACTOR,
            TEST_CORRELATION,
            race_now(),
        )
        .await
    });

    pg_support::wait_until_a_backend_blocks(&store.raw).await;
    release.notify_one();
    tokio::time::timeout(RACE_TIMEOUT, first)
        .await
        .expect("T1 must finish once released")
        .expect("its task must not panic")
        .expect("T1's named Put must commit");
    let after_t1 = artifact_counts(&store.raw).await;
    let second = tokio::time::timeout(RACE_TIMEOUT, second)
        .await
        .expect("T2 must reach a verdict once T1 releases")
        .expect("its task must not panic");
    let err = second.expect_err("publish must lose to T1's version bump");
    assert!(
        matches!(err, DomainError::StaleVersion(_)),
        "T2's conflict must be the version T1 moved, got {err:?}"
    );
    assert_invariants(
        &store.raw,
        &[],
        price_row_version(&store).await,
        after_t1,
        true,
    )
    .await;
    let listed = {
        let conn = store.db.conn().expect("conn");
        draft_window_repo::list(&conn, &scope(), &owner_rev(0))
            .await
            .expect("list draft operations")
    };
    assert_eq!(
        listed,
        vec![covering_create(
            COVER_WINDOW,
            "raceReplace",
            coverage_from(),
            None,
        )],
        "T1's Put is the winner's authorized draft operation"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires Docker (testcontainers)"]
async fn a_live_adjust_is_serialized_against_publish() {
    let (store, version) = seed_publishable_draft().await;
    let version = capture_empty_baseline(&store, version).await;
    let version = put_covering(&store, version, "cover").await;
    let registry = Arc::new(RegistryDouble::default());
    first_publish(&store, Arc::clone(&registry), version).await;
    close_materialized_covering_at_to(&store).await;
    install_eur_threshold(&store).await;
    let opened = store
        .plans
        .open_revision(&scope(), TENANT, PlanId::new(PLAN), race_stamp())
        .await
        .expect("open the successor T2 will publish");
    let draft_version = apply_on_store(
        &store,
        opened.revision,
        opened.row_version.get(),
        DraftWindowCommand::RefreshBaseline,
    )
    .await;
    let draft_version = apply_on_store(
        &store,
        opened.revision,
        draft_version,
        DraftWindowCommand::Put(covering_create(
            TAIL_WINDOW,
            "successorCover",
            coverage_to(),
            None,
        )),
    )
    .await;
    let cover_id = COVER_WINDOW;
    let seq = live_seq(&store, cover_id).await;
    let lengthened = utc_ymd_hms(2099, 10, 1, 0, 0, 0);

    let t2 = publish_service(
        DBProvider::<DbError>::new(store.pg.db().await),
        Arc::clone(&registry),
    )
    .await;
    let t1_windows = windows_on(
        DBProvider::<DbError>::new(store.pg.db().await),
        Arc::clone(&registry),
    );

    let wrote = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let first = {
        let db = store.pg.db().await;
        let (wrote, release) = (Arc::clone(&wrote), Arc::clone(&release));
        tokio::spawn(async move {
            let (_db, out) = db
                .in_transaction::<WindowMutationOutcome, DomainError, _>(move |txn| {
                    Box::pin(async move {
                        let outcome = t1_windows
                            .adjust_effective_to_in(
                                txn,
                                &race_ctx(),
                                &scope(),
                                TENANT,
                                cover_id,
                                Some(lengthened),
                                seq,
                                bss_pricing::api::rest::windows::verdict_json,
                                race_stamp(),
                            )
                            .await?;
                        must_commit(&outcome);
                        wrote.notify_one();
                        release.notified().await;
                        Ok(outcome)
                    })
                })
                .await;
            out
        })
    };
    wrote.notified().await;

    let second = tokio::spawn(async move {
        t2.commit(
            &race_ctx(),
            &scope(),
            TENANT,
            PlanPublishUnit::plan_content(PlanId::new(PLAN), opened.revision),
            RowVersion::new(draft_version),
            PublishAuthorization::auto_publishable(),
            ACTOR,
            TEST_CORRELATION,
            race_now(),
        )
        .await
    });

    pg_support::wait_until_a_backend_blocks(&store.raw).await;
    release.notify_one();
    tokio::time::timeout(RACE_TIMEOUT, first)
        .await
        .expect("T1 must finish once released")
        .expect("its task must not panic")
        .expect("T1's named adjust must commit");
    let after_t1 = artifact_counts(&store.raw).await;
    let second = tokio::time::timeout(RACE_TIMEOUT, second)
        .await
        .expect("T2 must reach a verdict once T1 releases")
        .expect("its task must not panic");
    let err = second.expect_err("publish must lose to T1's live lengthening");
    assert!(
        matches!(err, DomainError::WindowBaselineChanged(_)),
        "T2's conflict must be the baseline T1 moved, got {err:?}"
    );
    assert_invariants(
        &store.raw,
        &[cover_id.to_string()],
        price_row_version(&store).await,
        after_t1,
        true,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires Docker (testcontainers)"]
async fn a_row_key_edit_is_serialized_against_submit() {
    let (store, _version) = seed_publishable_draft().await;
    let record = store
        .prices
        .find(&scope(), TENANT, ROW)
        .await
        .expect("read the draft")
        .expect("the row exists");
    let expected = record.row_version;
    let mut content = record.content();
    content.row.amount_minor = Some(MinorAmount::new(10_900).expect("a non-negative amount"));

    let wrote = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let first = {
        let db = store.pg.db().await;
        let (wrote, release) = (Arc::clone(&wrote), Arc::clone(&release));
        tokio::spawn(async move {
            let (_db, out) = db
                .in_transaction::<bss_pricing::domain::price_record::PriceRecord, RepoError, _>(
                    move |txn| {
                        Box::pin(async move {
                            let updated = price_repo::update_draft_on(
                                txn,
                                &scope(),
                                TENANT,
                                ROW,
                                expected,
                                content,
                                race_stamp(),
                                None,
                            )
                            .await?;
                            wrote.notify_one();
                            release.notified().await;
                            Ok(updated)
                        })
                    },
                )
                .await;
            out
        })
    };
    wrote.notified().await;

    let db = store.pg.db().await;
    let second = tokio::spawn(async move {
        ApprovalService::new(DBProvider::<DbError>::new(db))
            .submit(
                &scope(),
                TENANT,
                PlanId::new(PLAN),
                Uuid::from_u128(0x_aa_01),
                json!({}),
                race_stamp(),
            )
            .await
    });

    pg_support::wait_until_a_backend_blocks(&store.raw).await;
    release.notify_one();
    tokio::time::timeout(RACE_TIMEOUT, first)
        .await
        .expect("T1 must finish once released")
        .expect("its task must not panic")
        .expect("T1's named update_draft must commit");
    let after_t1 = artifact_counts(&store.raw).await;
    let second = tokio::time::timeout(RACE_TIMEOUT, second)
        .await
        .expect("T2 must reach a verdict once T1 releases")
        .expect("its task must not panic");
    let version = price_row_version(&store).await;
    match second {
        Ok(_) => assert_invariants(&store.raw, &[], version, after_t1, false).await,
        Err(err) => {
            assert!(
                is_version_overlap_or_baseline(&err),
                "submit may succeed after T1 or conflict on T1's write, got {err:?}"
            );
            assert_invariants(&store.raw, &[], version, after_t1, true).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires Docker (testcontainers)"]
async fn concurrent_new_draft_windows_are_serialized() {
    let (store, version) = seed_publishable_draft().await;
    let version = capture_empty_baseline(&store, version).await;

    let wrote = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let first = {
        let db = store.pg.db().await;
        let (wrote, release) = (Arc::clone(&wrote), Arc::clone(&release));
        tokio::spawn(async move {
            let (_db, out) = db
                .in_transaction::<u64, DomainError, _>(move |txn| {
                    Box::pin(async move {
                        let next = draft_window::apply_command(
                            txn,
                            &scope(),
                            &owner_rev(0),
                            version,
                            DraftWindowCommand::Put(covering_create(
                                DRAFT_A,
                                "windowA",
                                coverage_from(),
                                None,
                            )),
                            race_stamp(),
                        )
                        .await?;
                        wrote.notify_one();
                        release.notified().await;
                        Ok(next)
                    })
                })
                .await;
            out
        })
    };
    wrote.notified().await;

    let db = store.pg.db().await;
    let second = tokio::spawn(async move {
        let (_db, out) = db
            .in_transaction::<u64, DomainError, _>(move |txn| {
                Box::pin(async move {
                    draft_window::apply_command(
                        txn,
                        &scope(),
                        &owner_rev(0),
                        version.saturating_add(1),
                        DraftWindowCommand::Put(covering_create(
                            DRAFT_B,
                            "windowB",
                            coverage_from(),
                            None,
                        )),
                        race_stamp(),
                    )
                    .await
                })
            })
            .await;
        out
    });

    pg_support::wait_until_a_backend_blocks(&store.raw).await;
    release.notify_one();
    tokio::time::timeout(RACE_TIMEOUT, first)
        .await
        .expect("T1 must finish once released")
        .expect("its task must not panic")
        .expect("T1's named Put A must commit");
    let after_t1 = artifact_counts(&store.raw).await;
    let second = tokio::time::timeout(RACE_TIMEOUT, second)
        .await
        .expect("T2 must reach a verdict once T1 releases")
        .expect("its task must not panic");
    let err = domain_from_tx(second).expect_err("Put B must not also land on the overlapping key");
    assert!(
        is_version_overlap_or_baseline(&err),
        "T2's conflict must be overlap or the version T1 moved, got {err:?}"
    );
    assert_invariants(
        &store.raw,
        &[],
        price_row_version(&store).await,
        after_t1,
        true,
    )
    .await;
    let listed = {
        let conn = store.db.conn().expect("conn");
        draft_window_repo::list(&conn, &scope(), &owner_rev(0))
            .await
            .expect("list draft operations")
    };
    assert_eq!(
        listed,
        vec![covering_create(DRAFT_A, "windowA", coverage_from(), None,)]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires Docker (testcontainers)"]
async fn two_overlapping_live_inserts_are_serialized() {
    let (store, version) = seed_publishable_draft().await;
    let version = capture_empty_baseline(&store, version).await;
    let version = put_covering(&store, version, "cover").await;
    let registry = Arc::new(RegistryDouble::default());
    first_publish(&store, Arc::clone(&registry), version).await;
    close_materialized_covering_at_to(&store).await;
    install_eur_threshold(&store).await;

    let t1_windows = windows_on(
        DBProvider::<DbError>::new(store.pg.db().await),
        Arc::clone(&registry),
    );
    let t2_windows = windows_on(
        DBProvider::<DbError>::new(store.pg.db().await),
        Arc::clone(&registry),
    );

    let wrote = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let first = {
        let db = store.pg.db().await;
        let (wrote, release) = (Arc::clone(&wrote), Arc::clone(&release));
        tokio::spawn(async move {
            let (_db, out) = db
                .in_transaction::<WindowMutationOutcome, DomainError, _>(move |txn| {
                    Box::pin(async move {
                        let outcome = t1_windows
                            .schedule_in(
                                txn,
                                &race_ctx(),
                                &scope(),
                                TENANT,
                                ROW,
                                LIVE_A,
                                coverage_to(),
                                Some(utc_ymd_hms(2099, 9, 16, 0, 0, 0)),
                                "windowA".to_owned(),
                                bss_pricing::api::rest::windows::verdict_json,
                                race_stamp(),
                            )
                            .await?;
                        must_commit(&outcome);
                        wrote.notify_one();
                        release.notified().await;
                        Ok(outcome)
                    })
                })
                .await;
            out
        })
    };
    wrote.notified().await;

    let second = tokio::spawn(async move {
        t2_windows
            .schedule(
                &race_ctx(),
                &scope(),
                TENANT,
                ROW,
                LIVE_B,
                utc_ymd_hms(2099, 9, 8, 0, 0, 0),
                Some(utc_ymd_hms(2099, 9, 20, 0, 0, 0)),
                "windowB".to_owned(),
                bss_pricing::api::rest::windows::verdict_json,
                race_stamp(),
            )
            .await
    });

    pg_support::wait_until_a_backend_blocks(&store.raw).await;
    release.notify_one();
    tokio::time::timeout(RACE_TIMEOUT, first)
        .await
        .expect("T1 must finish once released")
        .expect("its task must not panic")
        .expect("T1's named schedule must commit");
    let after_t1 = artifact_counts(&store.raw).await;
    let second = tokio::time::timeout(RACE_TIMEOUT, second)
        .await
        .expect("T2 must reach a verdict once T1 releases")
        .expect("its task must not panic");
    match second {
        Ok(WindowMutationOutcome::Committed(_)) => {
            panic!("T2 must not also commit an overlapping live window")
        }
        Ok(WindowMutationOutcome::SubmittedForApproval(_)) => {
            panic!("T2 submitted for approval instead of overlapping")
        }
        Err(err) => assert!(
            matches!(err, DomainError::WindowOverlap(_)),
            "T2's conflict must be the overlap T1 committed, got {err:?}"
        ),
    }
    let mut authorized = vec![COVER_WINDOW.to_string(), LIVE_A.to_string()];
    authorized.sort();
    assert_invariants(
        &store.raw,
        &authorized,
        price_row_version(&store).await,
        after_t1,
        true,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires Docker (testcontainers)"]
async fn a_delayed_commit_stamps_at_publish_from_the_post_wait_clock() {
    let (store, version) = seed_publishable_draft().await;
    let version = capture_empty_baseline(&store, version).await;
    let version = apply_on_store(
        &store,
        0,
        version,
        DraftWindowCommand::Put(covering_at_publish(COVER_WINDOW, "launch")),
    )
    .await;
    let registry = Arc::new(RegistryDouble::default());
    let t2 = publish_service(
        DBProvider::<DbError>::new(store.pg.db().await),
        Arc::clone(&registry),
    )
    .await;

    let wrote = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let first = {
        let db = store.pg.db().await;
        let (wrote, release) = (Arc::clone(&wrote), Arc::clone(&release));
        tokio::spawn(async move {
            let (_db, out) = db
                .in_transaction::<(), RepoError, _>(move |txn| {
                    Box::pin(async move {
                        window_guard_repo::acquire(txn, &scope(), TENANT, PLAN).await?;
                        wrote.notify_one();
                        release.notified().await;
                        Ok(())
                    })
                })
                .await;
            out
        })
    };
    wrote.notified().await;

    let second = tokio::spawn(async move {
        t2.commit(
            &race_ctx(),
            &scope(),
            TENANT,
            PlanPublishUnit::plan_content(PlanId::new(PLAN), 0),
            RowVersion::new(version),
            PublishAuthorization::auto_publishable(),
            ACTOR,
            TEST_CORRELATION,
            race_now(),
        )
        .await
    });

    pg_support::wait_until_a_backend_blocks(&store.raw).await;
    let released_at = OffsetDateTime::now_utc();
    release.notify_one();
    tokio::time::timeout(RACE_TIMEOUT, first)
        .await
        .expect("T1 must finish once released")
        .expect("its task must not panic")
        .expect("T1's parked guard must commit");
    let receipt = tokio::time::timeout(RACE_TIMEOUT, second)
        .await
        .expect("T2 must reach a verdict once T1 releases")
        .expect("its task must not panic")
        .expect("the delayed commit publishes");
    assert!(receipt.published_price_ids().contains(&ROW));

    let conn = store.db.conn().expect("conn");
    let windows = window_repo::list_for_plan(&conn, &scope(), TENANT, PlanId::new(PLAN))
        .await
        .expect("list committed windows");
    assert_eq!(windows.len(), 1, "exactly the authored create is written");
    assert_eq!(windows[0].window_id, COVER_WINDOW);
    assert_ne!(
        windows[0].effective_from,
        race_now(),
        "at_publish must not reuse the stale HTTP stamp after the guard wait"
    );
    let floor = bss_pricing::domain::instant::truncate_millis(released_at);
    assert!(
        windows[0].effective_from >= floor,
        "at_publish must stamp at or after the post-wait clock: got {}, floor {}",
        windows[0].effective_from,
        floor
    );
    assert_eq!(
        windows[0].effective_from,
        bss_pricing::domain::instant::truncate_millis(windows[0].effective_from),
        "the write instant is quantized to milliseconds"
    );
}
