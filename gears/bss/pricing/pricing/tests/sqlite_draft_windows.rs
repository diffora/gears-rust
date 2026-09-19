//! Revision-owned draft windows, captured baselines and the per-plan guard —
//! repository and schema, on the executed `SQLite` mirror.
//!
//! Overlap is **not** proved here. Draft overlap is judged against the composed
//! proposal (`domain::draft_window::compose_windows`), not across independent
//! revision rows or against live `pricing_price_window` exclusion.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use bss_pricing::domain::audit::AuditStamp;
use bss_pricing::domain::concurrency::RowVersion;
use bss_pricing::domain::draft_window::{
    DraftStart, DraftWindowAction, DraftWindowEntry, DraftWindowOwner, WindowBaseline,
};
use bss_pricing::domain::error::DomainError;
use bss_pricing::domain::instant::utc_ymd_hms;
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::scope_key::PlanId;
use bss_pricing::infra::draft_window::{self, DraftWindowCommand};
use bss_pricing::infra::storage::RepoError;
use bss_pricing::infra::storage::entity::{price, window_guard};
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::draft_window_repo;
use bss_pricing::infra::storage::repo::window_baseline_repo;
use bss_pricing::infra::storage::repo::window_guard_repo;
use bss_pricing::infra::storage::repo::window_repo::{self, NewWindow};
use bss_pricing::infra::storage::repo::{NewPlanDraft, PlanRepo};

use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use sea_orm_migration::MigratorTrait;
use time::OffsetDateTime;
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::{AccessScope, SecureDeleteExt, SecureInsertExt, SecureUpdateExt};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

mod common;

use bss_pricing::infra::storage::entity::plan;
use common::{exec, migrated_db, must_succeed};

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

async fn harness() -> DBProvider<DbError> {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    DBProvider::<DbError>::new(db)
}

async fn seed_price(
    provider: &DBProvider<DbError>,
    tenant_id: Uuid,
    plan_id: Uuid,
    price_id: Uuid,
) {
    let conn = provider.conn().expect("scoped connection");
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

async fn seeded_plan() -> (DBProvider<DbError>, PlanRepo) {
    let provider = harness().await;
    let repo = PlanRepo::new(provider.clone());
    repo.create_draft(&scope(), new_draft(PlanId::new(PLAN), TENANT))
        .await
        .expect("create revision 0");
    seed_price(&provider, TENANT, PLAN, ROW).await;
    (provider, repo)
}

async fn flip_state(provider: &DBProvider<DbError>, state: LifecycleState) {
    let conn = provider.conn().expect("conn");
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

// ---------------------------------------------------------------------------
// Repository: the world the store accepts
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_symbolic_create_round_trips_with_a_null_start() {
    let (provider, _plans) = seeded_plan().await;
    let conn = provider.conn().expect("conn");
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
async fn an_exact_create_round_trips_its_start() {
    let (provider, _plans) = seeded_plan().await;
    let conn = provider.conn().expect("conn");
    let window_id = Uuid::from_u128(0x_d7_02);
    let entry = DraftWindowEntry {
        operation_id: window_id,
        action: DraftWindowAction::Create {
            window_id,
            price_id: ROW,
            start: DraftStart::At(t(8)),
            effective_to: Some(t(20)),
        },
        reason_code: "scheduled launch".to_owned(),
    };
    draft_window_repo::put(&conn, &scope(), &owner(), &entry)
        .await
        .expect("put an exact create");
    let listed = draft_window_repo::list(&conn, &scope(), &owner())
        .await
        .expect("list");
    assert_eq!(listed, vec![entry]);
}

#[tokio::test]
async fn a_missing_owner_is_not_found() {
    let (provider, _plans) = seeded_plan().await;
    let conn = provider.conn().expect("conn");
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
async fn a_foreign_tenant_cannot_see_the_owner() {
    let (provider, _plans) = seeded_plan().await;
    let conn = provider.conn().expect("conn");
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
async fn a_price_on_another_plan_is_refused() {
    let (provider, plans) = seeded_plan().await;
    plans
        .create_draft(&scope(), new_draft(PlanId::new(OTHER_PLAN), TENANT))
        .await
        .expect("second plan");
    seed_price(&provider, TENANT, OTHER_PLAN, FOREIGN_ROW).await;
    let conn = provider.conn().expect("conn");
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
async fn a_second_operation_on_the_same_target_is_refused() {
    let (provider, _plans) = seeded_plan().await;
    let conn = provider.conn().expect("conn");
    let window_id = Uuid::from_u128(0x_d7_06);
    draft_window_repo::put(&conn, &scope(), &owner(), &symbolic_create(window_id))
        .await
        .expect("first create");
    let other_op = Uuid::from_u128(0xd716);
    let adjust = DraftWindowEntry {
        operation_id: other_op,
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
        err.to_string().contains("draft window")
            || err.to_string().contains("UNIQUE")
            || err.to_string().contains("unique")
            || matches!(err, RepoError::ConcurrentMutation { .. }),
        "duplicate target must be a uniqueness refusal, got: {err:?}"
    );
}

#[tokio::test]
async fn a_published_owner_rejects_put_and_keeps_the_row() {
    let (provider, _plans) = seeded_plan().await;
    let conn = provider.conn().expect("conn");
    let window_id = Uuid::from_u128(0x_d7_07);
    draft_window_repo::put(&conn, &scope(), &owner(), &symbolic_create(window_id))
        .await
        .expect("put while draft");
    flip_state(&provider, LifecycleState::Published).await;

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
async fn abandoning_a_revision_retains_its_intentions() {
    let (provider, plans) = seeded_plan().await;
    let conn = provider.conn().expect("conn");
    let window_id = Uuid::from_u128(0x_d7_08);
    draft_window_repo::put(&conn, &scope(), &owner(), &symbolic_create(window_id))
        .await
        .expect("put while draft");
    plans
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
async fn a_baseline_replace_round_trips() {
    let (provider, _plans) = seeded_plan().await;
    let conn = provider.conn().expect("conn");
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
async fn plan_create_inserts_a_guard_row_that_acquire_can_lock() {
    let (provider, _plans) = seeded_plan().await;
    let conn = provider.conn().expect("conn");
    window_guard_repo::acquire(&conn, &scope(), TENANT, PLAN)
        .await
        .expect("first acquire");
    window_guard_repo::acquire(&conn, &scope(), TENANT, PLAN)
        .await
        .expect("second acquire");
}

#[tokio::test]
async fn acquire_without_a_guard_row_is_an_error() {
    let provider = harness().await;
    let conn = provider.conn().expect("conn");
    let err = window_guard_repo::acquire(&conn, &scope(), TENANT, PLAN)
        .await
        .expect_err("no guard row");
    assert!(
        matches!(err, RepoError::NotFound { .. }),
        "zero affected rows must be NotFound, got: {err:?}"
    );
}

#[tokio::test]
async fn a_successor_revision_does_not_fail_the_guard_primary_key() {
    let (provider, plans) = seeded_plan().await;
    flip_state(&provider, LifecycleState::Published).await;
    let opened = plans
        .open_revision(&scope(), TENANT, PlanId::new(PLAN), stamp())
        .await
        .expect("successor must not collide on the per-plan guard");
    assert_eq!(opened.revision, 1);
    let conn = provider.conn().expect("conn");
    window_guard_repo::acquire(&conn, &scope(), TENANT, PLAN)
        .await
        .expect("the one guard row is still acquirable");
}

#[tokio::test]
async fn remove_drops_only_the_named_operation() {
    let (provider, _plans) = seeded_plan().await;
    let conn = provider.conn().expect("conn");
    let first = Uuid::from_u128(0x_d7_09);
    let second = Uuid::from_u128(0x_d7_0a);
    draft_window_repo::put(&conn, &scope(), &owner(), &symbolic_create(first))
        .await
        .expect("first");
    draft_window_repo::put(&conn, &scope(), &owner(), &symbolic_create(second))
        .await
        .expect("second");
    draft_window_repo::remove(&conn, &scope(), &owner(), first)
        .await
        .expect("remove first");
    let listed = draft_window_repo::list(&conn, &scope(), &owner())
        .await
        .expect("list");
    assert_eq!(listed, vec![symbolic_create(second)]);
}

// ---------------------------------------------------------------------------
// Schema: CHECKs and draft-owner triggers, past the repository
// ---------------------------------------------------------------------------

fn tenant_s() -> String {
    TENANT.to_string()
}

#[tokio::test]
async fn at_publish_cannot_carry_an_authored_start() {
    let conn = migrated_db().await;
    must_succeed(
        &conn,
        &format!(
            "INSERT INTO pricing_plan (plan_id, revision, tenant_id, lifecycle_state, created_by, created_at_utc, sku_id)
             VALUES ('{PLAN}', 0, '{}', 'draft', '{ACTOR}', '2026-09-18 10:00:00 +00:00', '{SKU}')",
            tenant_s()
        ),
    )
    .await;
    let graph = common::seed_charge_graph_sql(
        &conn,
        &common::SqlGraphSeed {
            lifecycle_state: "draft",
            created_by: &ACTOR.to_string(),
            created_at_utc: "2026-09-18 10:00:00 +00:00",
            ..common::SqlGraphSeed::new(
                &tenant_s(),
                &PLAN.to_string(),
                &PHASE.to_string(),
                &SKU.to_string(),
            )
        },
    )
    .await;
    must_succeed(
        &conn,
        &format!(
            "INSERT INTO pricing_price (price_id, tenant_id, plan_id, plan_revision, charge_line_id, line_version_id, market_price_id, amount_minor, lifecycle_state, created_by, created_at_utc)
             VALUES ('{ROW}', '{}', '{PLAN}', 0, '{}', '{}', '{}', 1000, 'draft', '{ACTOR}', '2026-09-18 10:00:00 +00:00')",
            tenant_s(), graph.charge_line_id, graph.line_version_id, graph.market_price_id
        ),
    )
    .await;
    let err = exec(
        &conn,
        &format!(
            "INSERT INTO pricing_draft_window (
                tenant_id, plan_id, plan_revision, operation_id, target_window_id,
                action, price_id, start_kind, effective_from, effective_to, reason_code)
             VALUES ('{}', '{PLAN}', 0, '{ROW}', '{ROW}',
                'create', '{ROW}', 'at_publish', '2099-09-08T00:00:00+00:00', NULL, 'launch')",
            tenant_s()
        ),
    )
    .await
    .expect_err("at_publish plus an instant is incoherent");
    assert!(
        err.to_string().contains("chk_pricing_draft_window_shape"),
        "must be the shape CHECK, got: {err}"
    );
}

#[tokio::test]
async fn a_non_draft_revision_refuses_insert() {
    let conn = migrated_db().await;
    must_succeed(
        &conn,
        &format!(
            "INSERT INTO pricing_plan (plan_id, revision, tenant_id, lifecycle_state, created_by, created_at_utc, sku_id)
             VALUES ('{PLAN}', 0, '{}', 'draft', '{ACTOR}', '2026-09-18 10:00:00 +00:00', '{SKU}')",
            tenant_s()
        ),
    )
    .await;
    must_succeed(
        &conn,
        &format!(
            "UPDATE pricing_plan SET lifecycle_state = 'published' WHERE plan_id = '{PLAN}' AND revision = 0"
        ),
    )
    .await;
    let graph = common::seed_charge_graph_sql(
        &conn,
        &common::SqlGraphSeed {
            lifecycle_state: "published",
            created_by: &ACTOR.to_string(),
            created_at_utc: "2026-09-18 10:00:00 +00:00",
            ..common::SqlGraphSeed::new(
                &tenant_s(),
                &PLAN.to_string(),
                &PHASE.to_string(),
                &SKU.to_string(),
            )
        },
    )
    .await;
    must_succeed(
        &conn,
        &format!(
            "INSERT INTO pricing_price (price_id, tenant_id, plan_id, plan_revision, charge_line_id, line_version_id, market_price_id, amount_minor, lifecycle_state, created_by, created_at_utc)
             VALUES ('{ROW}', '{}', '{PLAN}', 0, '{}', '{}', '{}', 1000, 'published', '{ACTOR}', '2026-09-18 10:00:00 +00:00')",
            tenant_s(), graph.charge_line_id, graph.line_version_id, graph.market_price_id
        ),
    )
    .await;
    let err = exec(
        &conn,
        &format!(
            "INSERT INTO pricing_draft_window (
                tenant_id, plan_id, plan_revision, operation_id, target_window_id,
                action, price_id, start_kind, effective_from, effective_to, reason_code)
             VALUES ('{}', '{PLAN}', 0, '{ROW}', '{ROW}',
                'create', '{ROW}', 'at_publish', NULL, NULL, 'launch')",
            tenant_s()
        ),
    )
    .await
    .expect_err("published owner");
    assert!(
        err.to_string().contains("pricing_draft_window"),
        "must name the draft-window guard, got: {err}"
    );
}

#[tokio::test]
async fn apply_command_without_a_guard_row_is_an_error() {
    let (provider, _plans) = seeded_plan().await;
    let conn = provider.conn().expect("conn");
    window_guard::Entity::delete_many()
        .secure()
        .scope_with(&scope())
        .exec(&conn)
        .await
        .expect("drop the guard");
    let err = draft_window::apply_command(
        &conn,
        &scope(),
        &owner(),
        0,
        DraftWindowCommand::Put(symbolic_create(Uuid::from_u128(0x_d7_b1))),
        stamp(),
    )
    .await
    .expect_err("no guard row");
    assert!(
        matches!(err, DomainError::NotFound { .. }),
        "zero affected rows must be NotFound, got: {err:?}"
    );
}

#[tokio::test]
async fn sequential_draft_puts_serialize_as_success_then_success() {
    let (provider, _plans) = seeded_plan().await;
    let conn = provider.conn().expect("conn");
    let first = Uuid::from_u128(0x_d7_c1);
    let second = Uuid::from_u128(0x_d7_c2);
    let first_entry = DraftWindowEntry {
        operation_id: first,
        action: DraftWindowAction::Create {
            window_id: first,
            price_id: ROW,
            start: DraftStart::AtPublish,
            effective_to: Some(t(10)),
        },
        reason_code: "launch".to_owned(),
    };
    let second_entry = DraftWindowEntry {
        operation_id: second,
        action: DraftWindowAction::Create {
            window_id: second,
            price_id: ROW,
            start: DraftStart::At(t(10)),
            effective_to: None,
        },
        reason_code: "successor".to_owned(),
    };
    let v1 = draft_window::apply_command(
        &conn,
        &scope(),
        &owner(),
        0,
        DraftWindowCommand::Put(first_entry),
        stamp(),
    )
    .await
    .expect("first put");
    draft_window::apply_command(
        &conn,
        &scope(),
        &owner(),
        v1,
        DraftWindowCommand::Put(second_entry),
        stamp(),
    )
    .await
    .expect("second put");
    let listed = draft_window_repo::list(&conn, &scope(), &owner())
        .await
        .expect("list");
    assert_eq!(listed.len(), 2);
}

#[tokio::test]
async fn a_new_live_window_conflicts_with_an_empty_captured_baseline() {
    let (provider, _plans) = seeded_plan().await;
    let conn = provider.conn().expect("conn");
    window_repo::schedule(
        &conn,
        &scope(),
        NewWindow {
            window_id: Uuid::from_u128(0x_e1),
            tenant_id: TENANT,
            price_id: ROW,
            effective_from: t(8),
            effective_to: None,
            reason_code: "live membership".to_owned(),
        },
        stamp(),
    )
    .await
    .expect("schedule a live window the captured baseline does not name");
    let err = draft_window::apply_command(
        &conn,
        &scope(),
        &owner(),
        0,
        DraftWindowCommand::Put(symbolic_create(Uuid::from_u128(0x_d7_c3))),
        stamp(),
    )
    .await
    .expect_err("new live membership must conflict");
    assert!(
        matches!(err, DomainError::WindowBaselineChanged(_)),
        "WINDOW_BASELINE_CHANGED, got: {err:?}"
    );
}
