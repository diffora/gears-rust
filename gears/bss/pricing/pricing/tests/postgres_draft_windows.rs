//! Revision-owned draft windows, captured baselines and the per-plan guard —
//! repository and schema, on Postgres.
//!
//! Mirrors `sqlite_draft_windows.rs`. Overlap is judged against the composed
//! proposal, not across independent revisions.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod pg_support;

use bss_pricing::domain::audit::AuditStamp;
use bss_pricing::domain::concurrency::RowVersion;
use bss_pricing::domain::draft_window::{
    DraftStart, DraftWindowAction, DraftWindowEntry, DraftWindowOwner, WindowBaseline,
};
use bss_pricing::domain::instant::utc_ymd_hms;
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::scope_key::PlanId;
use bss_pricing::infra::storage::RepoError;
use bss_pricing::infra::storage::entity::{plan, price};
use bss_pricing::infra::storage::repo::draft_window_repo;
use bss_pricing::infra::storage::repo::window_baseline_repo;
use bss_pricing::infra::storage::repo::window_guard_repo;
use bss_pricing::infra::storage::repo::{NewPlanDraft, PlanRepo};

use pg_support::Pg;
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, EntityTrait, Statement,
};
use time::OffsetDateTime;
use toolkit_db::secure::{AccessScope, SecureInsertExt, SecureUpdateExt};
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
        plan_name: None,
        plan_id,
        tenant_id,
        created_by: ACTOR,
        created_at_utc: t(1),
        sku_id: SKU,
        plan_tier: None,
        billing_cycle: None,
        frequency: None,
        plan_tier_override: false,
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
    db: DBProvider<DbError>,
    plans: PlanRepo,
    raw: DatabaseConnection,
}

async fn store() -> Store {
    let pg = Pg::applied().await;
    let db = DBProvider::<DbError>::new(pg.db().await);
    let plans = PlanRepo::new(db.clone());
    let raw = pg.raw().await;
    Store { db, plans, raw }
}

async fn seed_price(store: &Store, tenant_id: Uuid, plan_id: Uuid, price_id: Uuid) {
    let conn = store.db.conn().expect("scoped connection");
    let row_scope = AccessScope::for_tenant(tenant_id);
    let row = price::ActiveModel {
        price_id: Set(price_id),
        tenant_id: Set(tenant_id),
        plan_id: Set(plan_id),
        sku_id: Set(SKU),
        currency: Set("USD".to_owned()),
        region: Set("EU".to_owned()),
        phase: Set(PHASE),
        charge_kind: Set("recurring".to_owned()),
        amount_minor: Set(Some(1_000)),
        model_kind: Set(Some("flat".to_owned())),
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
        operation_id: Uuid::from_u128(0x_d7_16),
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
    window_baseline_repo::replace(&conn, &scope(), &owner(), &[captured.clone()])
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
