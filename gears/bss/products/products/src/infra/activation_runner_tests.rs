//! Sweep probes: a due row is claimed and deferred while the host refuses
//! `PreAuthorized` — fail-closed, no invented consume-at-schedule.
#![allow(clippy::expect_used)]

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait as _, Condition, EntityTrait as _};
use sea_orm_migration::MigratorTrait;
use tokio_util::sync::CancellationToken;
use toolkit_db::outbox::{Outbox, OutboxHandle, Partitions, outbox_migrations_with_prefix};
use toolkit_db::secure::{AccessScope, SecureEntityExt as _, SecureUpdateExt as _};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

use super::{ActivationContext, sweep};
use crate::config::IDEMPOTENCY_RETENTION_FLOOR_HOURS;
use crate::domain::activation::{AttemptBudget, ClaimLease};
use crate::domain::concurrency::InternalRevision;
use crate::domain::governance::{ApprovalId, EntityRef, GateSubject};
use crate::domain::materiality::{
    MaterialAct, MaterialityEvaluator, MaterialityPolicy, Resolution,
};
use crate::domain::retention::RetentionHold;
use crate::infra::events;
use crate::infra::storage::entity::{approval, audit_log, product, sku};
use crate::infra::storage::migrations::Migrator;
use crate::infra::storage::repo::{
    NewApproval, NewProduct, NewScheduledTransition, NewSku, consume_approval, find_product,
    find_scheduled_transition, find_sku, insert_product, insert_scheduled_transition, insert_sku,
    submit_approval, write_replaced_by,
};

const TENANT: Uuid = Uuid::from_u128(0x7e_42);
const ENTITY: Uuid = Uuid::from_u128(0xdd_11);
const TRANSITION: Uuid = Uuid::from_u128(0xaa_01);
const APPROVAL: Uuid = Uuid::from_u128(0xbb_02);
const ACTOR: Uuid = Uuid::from_u128(0xcc_03);

struct Harness {
    db: DBProvider<DbError>,
    #[allow(dead_code)]
    outbox_handle: OutboxHandle,
}

async fn harness() -> Harness {
    let opts = ConnectOpts {
        max_conns: Some(4),
        min_conns: Some(1),
        ..Default::default()
    };
    let db = connect_db("sqlite::memory:", opts)
        .await
        .expect("connect in-memory sqlite");
    toolkit_db::migration_runner::run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("boot the migration chain");
    toolkit_db::migration_runner::run_migrations_for_testing(
        &db,
        outbox_migrations_with_prefix(events::OUTBOX_TABLE_PREFIX).expect("prefix"),
    )
    .await
    .expect("outbox migrate");
    let outbox_handle = Outbox::builder(db.clone())
        .table_prefix(events::OUTBOX_TABLE_PREFIX)
        .expect("prefix")
        .queue(events::QUEUE_NAME, Partitions::of(events::PARTITIONS))
        .leased(events::PendingBrokerProducer)
        .start()
        .await
        .expect("start the outbox");
    let db = DBProvider::<DbError>::new(db);
    // The fixtures are `product` SKUs: they must carry both Finance codes to
    // publish (P-D-145), and a `bundle` would need P-D-02's acknowledgment
    // on its record instead (P-D-146).
    crate::test_support::seed_finance_codes(&db, TENANT).await;
    Harness { db, outbox_handle }
}

fn context(harness: &Harness) -> ActivationContext {
    ActivationContext {
        db: harness.db.clone(),
        lease: ClaimLease {
            ttl: chrono::Duration::seconds(60),
        },
        budget: AttemptBudget { max: 5 },
        retirement_held_alert_hours: 72,
        sink: crate::infra::broker::EventSink::Interim(Arc::clone(harness.outbox_handle.outbox())),
        idempotency_retention_hours: IDEMPOTENCY_RETENTION_FLOOR_HOURS,
        reference_freshness: crate::config::ProductsConfig::default().reference_freshness(),
        usage_type_resolver: crate::test_support::resolved_usage_types(),
    }
}

#[tokio::test]
async fn a_due_row_defers_while_the_host_refuses_preauthorized() {
    let harness = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    let now = Utc.with_ymd_and_hms(2026, 9, 4, 12, 0, 0).unwrap();
    {
        let conn = harness.db.conn().expect("scoped connection");
        insert_scheduled_transition(
            &conn,
            &scope,
            &NewScheduledTransition {
                transition_id: TRANSITION,
                tenant_id: TENANT,
                entity_kind: "sku".to_owned(),
                entity_id: ENTITY,
                kind: "publish".to_owned(),
                at: now - chrono::Duration::hours(1),
                approval_ref: APPROVAL,
                retirement_reason: None,
                now,
            },
        )
        .await
        .expect("insert");
    }

    let ctx = context(&harness);
    sweep(&ctx, ACTOR, now, &CancellationToken::new())
        .await
        .expect("sweep");

    let conn = ctx.db.conn().expect("reopen");
    let row = find_scheduled_transition(&conn, &scope, TENANT, TRANSITION)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(row.state, "deferred");
    assert_eq!(row.attempt, 1, "the claim spent one");
    assert!(
        row.outcome_reason
            .as_deref()
            .is_some_and(|r| r.contains("activation gate refused")),
        "host Refused is the transient arm, not a silent apply: {:?}",
        row.outcome_reason
    );
}

const PRODUCT: Uuid = Uuid::from_u128(0xdd_10);
const BRAND: Uuid = Uuid::from_u128(0xdd_b1);
const SKU: Uuid = Uuid::from_u128(0xdd_21);
const SEEDED_TRANSITION: Uuid = Uuid::from_u128(0xaa_11);
const SEEDED_APPROVAL: Uuid = Uuid::from_u128(0xbb_12);

/// A published parent, a draft SKU (a usage SKU when `metering` names the
/// pair), a consumed approval pinned at revision 1 and a due publish row.
async fn seed_scheduled_publish(
    harness: &Harness,
    scope: &AccessScope,
    now: chrono::DateTime<Utc>,
    metering: Option<(&str, &str)>,
) {
    let conn = harness.db.conn().expect("scoped connection");
    insert_product(
        &conn,
        scope,
        NewProduct {
            product_id: PRODUCT,
            tenant_id: TENANT,
            brand_id: BRAND,
            name: "Fibre 500".to_owned(),
            name_normalized: "fibre 500".to_owned(),
            product_code: Some("FIBRE-500".to_owned()),
            region_scope: String::new(),
            brand_scope: String::new(),
            created_by: "principal:author-1".to_owned(),
            created_at: now,
            cloned_from: None,
            cloned_from_version: None,
        },
    )
    .await
    .expect("insert the parent");
    // Walk the admitted `draft → published` edge without a version freeze:
    // the SKU door only asks that the parent *state* is published.
    product::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(product::Column::LifecycleState, Expr::value("published"))
        .col_expr(product::Column::InternalRevision, Expr::value(2_i64))
        .col_expr(product::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(product::Column::TenantId.eq(TENANT))
                .add(product::Column::ProductId.eq(PRODUCT)),
        )
        .exec(&conn)
        .await
        .expect("the floor admits draft -> published");
    if let Some((unit, _)) = metering {
        crate::infra::storage::repo::insert_recognized_member(
            &conn,
            scope,
            TENANT,
            crate::domain::recognized::SetKind::MeteringUnit,
            unit,
            None,
            None,
            now,
        )
        .await
        .expect("seed the unit");
    }
    insert_sku(
        &conn,
        scope,
        NewSku {
            sku_id: SKU,
            tenant_id: TENANT,
            product_id: PRODUCT,
            sku_code: "FIBRE-500-1".to_owned(),
            region_scope: String::new(),
            brand_scope: String::new(),
            created_by: "principal:author-1".to_owned(),
            created_at: now,
            cloned_from: None,
            cloned_from_version: None,
            sku_type: "product".to_owned(),
            sellable: true,
            plan_tier: "standard".to_owned(),
            tax_category_ref: Some("TC-STD".to_owned()),
            gl_code_ref: Some("GL-4000".to_owned()),
            metering_unit: metering.map(|(unit, _)| unit.to_owned()),
            usage_type_ref: metering.map(|(_, usage_type_ref)| usage_type_ref.to_owned()),
        },
    )
    .await
    .expect("insert the draft SKU");

    let subject = GateSubject::entity_publish(
        EntityRef {
            tenant_id: TENANT,
            entity_kind: bss_products_sdk::models::EntityKind::Sku,
            entity_id: SKU,
        },
        InternalRevision::new(1),
    );
    let policy = MaterialityPolicy::default();
    let evaluator = MaterialityEvaluator::new(Resolution::Resolved(&policy));
    let act = MaterialAct::PolicyMutation;
    submit_approval(
        &conn,
        scope,
        NewApproval {
            approval_id: ApprovalId::new(SEEDED_APPROVAL),
            subject: &subject,
            internal_revision: 1,
            content_snapshot: "{}",
            diff_basis: None,
            act: &act,
            evaluator,
            finance_material: false,
            approver_count: 2,
            submitter: ACTOR,
            author_override_ack: None,
            override_conditions: Vec::new(),
        },
        now,
    )
    .await
    .expect("submit");
    approval::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(approval::Column::State, Expr::value("satisfied".to_owned()))
        .filter(
            Condition::all()
                .add(approval::Column::TenantId.eq(TENANT))
                .add(approval::Column::ApprovalId.eq(SEEDED_APPROVAL)),
        )
        .exec(&conn)
        .await
        .expect("satisfy");
    consume_approval(&conn, scope, TENANT, ApprovalId::new(SEEDED_APPROVAL), now)
        .await
        .expect("consume");

    insert_scheduled_transition(
        &conn,
        scope,
        &NewScheduledTransition {
            transition_id: SEEDED_TRANSITION,
            tenant_id: TENANT,
            entity_kind: "sku".to_owned(),
            entity_id: SKU,
            kind: "publish".to_owned(),
            at: now - chrono::Duration::hours(1),
            approval_ref: SEEDED_APPROVAL,
            retirement_reason: None,
            now,
        },
    )
    .await
    .expect("insert the pin");
}

/// A real consumed approval exists in B's store. The runner must drive the
/// ordinary Foundation publish door — not mark `applied` without a publish,
/// and not invent consume-at-schedule.
#[tokio::test]
async fn a_seeded_consumed_approval_drives_the_foundation_publish_door() {
    let harness = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    let now = Utc.with_ymd_and_hms(2026, 9, 4, 12, 0, 0).unwrap();
    seed_scheduled_publish(&harness, &scope, now, None).await;

    let ctx = context(&harness);
    sweep(&ctx, ACTOR, now, &CancellationToken::new())
        .await
        .expect("sweep");

    let conn = ctx.db.conn().expect("reopen");
    let row = find_scheduled_transition(&conn, &scope, TENANT, SEEDED_TRANSITION)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(
        row.state, "applied",
        "a seeded consumed pin must apply, not defer: {:?}",
        row.outcome_reason
    );
    let sku = find_sku(&conn, &scope, TENANT, SKU)
        .await
        .expect("find sku")
        .expect("sku");
    assert_eq!(
        sku.lifecycle_state,
        bss_products_sdk::models::LifecycleState::Published,
        "the runner drives the Foundation publish door; Applied without a publish is a privileged path"
    );
}

/// **`USAGE_TYPE_UNAVAILABLE` on the scheduled lane leaves the transition
/// `deferred`, not `failed`, and its pinned approval survives** (`03` §6;
/// P-D-157). The runner resolves a usage SKU's ref before its publish, as
/// the REST door does: a collector that does not answer defers the row and
/// publishes nothing; once it answers, the next sweep applies the same pin.
#[tokio::test]
async fn a_usage_skus_scheduled_publish_defers_while_the_collector_is_unavailable_and_applies_once_it_answers()
 {
    let harness = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    let now = Utc.with_ymd_and_hms(2026, 9, 4, 12, 0, 0).unwrap();
    seed_scheduled_publish(&harness, &scope, now, Some(("gib_month", "usage:storage"))).await;

    let unavailable = Arc::new(crate::test_support::StubUsageTypes::always(
        crate::domain::recognized::UsageTypeAnswer::Unavailable,
    ));
    let mut ctx = context(&harness);
    ctx.usage_type_resolver =
        Arc::clone(&unavailable) as Arc<dyn crate::infra::usage_types::UsageTypeResolver>;
    sweep(&ctx, ACTOR, now, &CancellationToken::new())
        .await
        .expect("sweep");
    assert_eq!(
        unavailable.asked.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the lane asked the collector once"
    );
    {
        let conn = ctx.db.conn().expect("reopen");
        let row = find_scheduled_transition(&conn, &scope, TENANT, SEEDED_TRANSITION)
            .await
            .expect("find")
            .expect("row");
        assert_eq!(
            row.state, "deferred",
            "a transient dependency defers, never fails: {:?}",
            row.outcome_reason
        );
        assert!(
            row.outcome_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("USAGE_TYPE_UNAVAILABLE")),
            "the deferral names the collector's silence: {:?}",
            row.outcome_reason
        );
        let sku = find_sku(&conn, &scope, TENANT, SKU)
            .await
            .expect("find sku")
            .expect("sku");
        assert_eq!(
            sku.lifecycle_state,
            bss_products_sdk::models::LifecycleState::Draft,
            "nothing published under an unresolved ref"
        );
        let pinned = crate::infra::storage::repo::read_approval(
            &conn,
            &scope,
            TENANT,
            ApprovalId::new(SEEDED_APPROVAL),
        )
        .await
        .expect("read")
        .expect("the pinned approval");
        assert_eq!(pinned.state, "consumed", "the pin survives the deferral");
    }

    // The collector answers on the next sweep: the same pin applies.
    let later = now + chrono::Duration::hours(2);
    let ctx = context(&harness);
    sweep(&ctx, ACTOR, later, &CancellationToken::new())
        .await
        .expect("second sweep");
    let conn = ctx.db.conn().expect("reopen");
    let row = find_scheduled_transition(&conn, &scope, TENANT, SEEDED_TRANSITION)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(row.state, "applied", "{:?}", row.outcome_reason);
    let sku = find_sku(&conn, &scope, TENANT, SKU)
        .await
        .expect("find sku")
        .expect("sku");
    assert_eq!(
        sku.lifecycle_state,
        bss_products_sdk::models::LifecycleState::Published,
        "the deferred row publishes once the collector answers"
    );
}

const RETIRE_SKU: Uuid = Uuid::from_u128(0xdd_31);
const RETIRE_TRANSITION: Uuid = Uuid::from_u128(0xaa_21);
const RETIRE_APPROVAL: Uuid = Uuid::from_u128(0xbb_22);
const ORPHAN_CHILD: Uuid = Uuid::from_u128(0xdd_41);
const ORPHAN_TRANSITION: Uuid = Uuid::from_u128(0xaa_31);
const ORPHAN_APPROVAL: Uuid = Uuid::from_u128(0x00bb_0032);

/// Walk admitted edges on a SKU head. Starts at revision 1 (`draft`).
async fn walk_sku(
    conn: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    sku_id: Uuid,
    now: chrono::DateTime<Utc>,
    states: &[&str],
) {
    for (step, state) in states.iter().enumerate() {
        let next = 1 + i64::try_from(step).expect("step") + 1;
        sku::Entity::update_many()
            .secure()
            .scope_with(scope)
            .col_expr(sku::Column::LifecycleState, Expr::value(*state))
            .col_expr(sku::Column::InternalRevision, Expr::value(next))
            .col_expr(sku::Column::UpdatedAt, Expr::value(now))
            .filter(
                Condition::all()
                    .add(sku::Column::TenantId.eq(TENANT))
                    .add(sku::Column::SkuId.eq(sku_id)),
            )
            .exec(conn)
            .await
            .unwrap_or_else(|e| panic!("the floor admits the edge into `{state}`: {e}"));
    }
}

/// A registered producer whose fresh watermark omits every SKU — `FreshZero`.
async fn seed_fresh_zero(
    conn: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    now: chrono::DateTime<Utc>,
) {
    crate::infra::storage::repo::register_reference_producer(
        conn, scope, TENANT, "pricing", None, now,
    )
    .await
    .expect("register a producer");
    crate::infra::storage::repo::post_reference_watermark(
        conn,
        scope,
        TENANT,
        crate::infra::storage::repo::PostedWatermark {
            producer: "pricing",
            watermark_at: now,
            posted_at: now,
            set_hash: "0000000000000000000000000000000000000000000000000000000000000000",
            members: &[],
        },
    )
    .await
    .expect("post a fresh empty set");
}

#[tokio::test]
async fn a_seeded_consumed_approval_flips_deprecated_sku_to_retired() {
    let harness = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    let now = Utc.with_ymd_and_hms(2026, 9, 4, 12, 0, 0).unwrap();
    {
        let conn = harness.db.conn().expect("scoped connection");
        insert_product(
            &conn,
            &scope,
            NewProduct {
                product_id: PRODUCT,
                tenant_id: TENANT,
                brand_id: BRAND,
                name: "Fibre 500".to_owned(),
                name_normalized: "fibre 500".to_owned(),
                product_code: Some("FIBRE-500".to_owned()),
                region_scope: String::new(),
                brand_scope: String::new(),
                created_by: "principal:author-1".to_owned(),
                created_at: now,
                cloned_from: None,
                cloned_from_version: None,
            },
        )
        .await
        .expect("insert the parent");
        product::Entity::update_many()
            .secure()
            .scope_with(&scope)
            .col_expr(product::Column::LifecycleState, Expr::value("published"))
            .col_expr(product::Column::InternalRevision, Expr::value(2_i64))
            .col_expr(product::Column::UpdatedAt, Expr::value(now))
            .filter(
                Condition::all()
                    .add(product::Column::TenantId.eq(TENANT))
                    .add(product::Column::ProductId.eq(PRODUCT)),
            )
            .exec(&conn)
            .await
            .expect("parent published");
        insert_sku(
            &conn,
            &scope,
            NewSku {
                sku_id: RETIRE_SKU,
                tenant_id: TENANT,
                product_id: PRODUCT,
                sku_code: "FIBRE-500-R".to_owned(),
                region_scope: String::new(),
                brand_scope: String::new(),
                created_by: "principal:author-1".to_owned(),
                created_at: now,
                cloned_from: None,
                cloned_from_version: None,
                sku_type: "product".to_owned(),
                sellable: true,
                plan_tier: "standard".to_owned(),
                tax_category_ref: Some("TC-STD".to_owned()),
                gl_code_ref: Some("GL-4000".to_owned()),
                metering_unit: None,
                usage_type_ref: None,
            },
        )
        .await
        .expect("insert sku");
        walk_sku(&conn, &scope, RETIRE_SKU, now, &["published", "deprecated"]).await;
        seed_fresh_zero(&conn, &scope, now).await;

        let subject = GateSubject::entity_publish(
            EntityRef {
                tenant_id: TENANT,
                entity_kind: bss_products_sdk::models::EntityKind::Sku,
                entity_id: RETIRE_SKU,
            },
            InternalRevision::new(1),
        );
        let policy = MaterialityPolicy::default();
        let evaluator = MaterialityEvaluator::new(Resolution::Resolved(&policy));
        let act = MaterialAct::PolicyMutation;
        submit_approval(
            &conn,
            &scope,
            NewApproval {
                approval_id: ApprovalId::new(RETIRE_APPROVAL),
                subject: &subject,
                internal_revision: 3,
                content_snapshot: "{}",
                diff_basis: None,
                act: &act,
                evaluator,
                finance_material: false,
                approver_count: 2,
                submitter: ACTOR,
                author_override_ack: None,
                override_conditions: Vec::new(),
            },
            now,
        )
        .await
        .expect("submit");
        approval::Entity::update_many()
            .secure()
            .scope_with(&scope)
            .col_expr(approval::Column::State, Expr::value("satisfied".to_owned()))
            .filter(
                Condition::all()
                    .add(approval::Column::TenantId.eq(TENANT))
                    .add(approval::Column::ApprovalId.eq(RETIRE_APPROVAL)),
            )
            .exec(&conn)
            .await
            .expect("satisfy");
        consume_approval(&conn, &scope, TENANT, ApprovalId::new(RETIRE_APPROVAL), now)
            .await
            .expect("consume");
        insert_scheduled_transition(
            &conn,
            &scope,
            &NewScheduledTransition {
                transition_id: RETIRE_TRANSITION,
                tenant_id: TENANT,
                entity_kind: "sku".to_owned(),
                entity_id: RETIRE_SKU,
                kind: "retire".to_owned(),
                at: now - chrono::Duration::hours(1),
                approval_ref: RETIRE_APPROVAL,
                retirement_reason: Some("end of life".to_owned()),
                now,
            },
        )
        .await
        .expect("insert the pin");
    }

    let ctx = context(&harness);
    sweep(&ctx, ACTOR, now, &CancellationToken::new())
        .await
        .expect("sweep");

    let conn = ctx.db.conn().expect("reopen");
    let row = find_scheduled_transition(&conn, &scope, TENANT, RETIRE_TRANSITION)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(
        row.state, "applied",
        "retire flip must apply: {:?}",
        row.outcome_reason
    );
    let sku = find_sku(&conn, &scope, TENANT, RETIRE_SKU)
        .await
        .expect("find sku")
        .expect("sku");
    assert_eq!(
        sku.lifecycle_state,
        bss_products_sdk::models::LifecycleState::Retired,
        "the runner drives deprecated -> retired"
    );
    let applied = audit_log::Entity::find()
        .secure()
        .scope_with(&scope)
        .filter(Condition::all().add(audit_log::Column::Action.eq("activation.applied")))
        .one(&conn)
        .await
        .expect("query");
    assert!(
        applied.is_some(),
        "the applied finish leaves an audit row, not only a state change"
    );
}

#[tokio::test]
async fn a_product_retire_defers_when_a_published_child_would_orphan() {
    let harness = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    let now = Utc.with_ymd_and_hms(2026, 9, 4, 12, 0, 0).unwrap();
    {
        let conn = harness.db.conn().expect("scoped connection");
        insert_product(
            &conn,
            &scope,
            NewProduct {
                product_id: PRODUCT,
                tenant_id: TENANT,
                brand_id: BRAND,
                name: "Fibre 500".to_owned(),
                name_normalized: "fibre 500".to_owned(),
                product_code: Some("FIBRE-500".to_owned()),
                region_scope: String::new(),
                brand_scope: String::new(),
                created_by: "principal:author-1".to_owned(),
                created_at: now,
                cloned_from: None,
                cloned_from_version: None,
            },
        )
        .await
        .expect("insert the parent");
        for (step, state) in ["published", "deprecated"].iter().enumerate() {
            let next = 1 + i64::try_from(step).expect("step") + 1;
            product::Entity::update_many()
                .secure()
                .scope_with(&scope)
                .col_expr(product::Column::LifecycleState, Expr::value(*state))
                .col_expr(product::Column::InternalRevision, Expr::value(next))
                .col_expr(product::Column::UpdatedAt, Expr::value(now))
                .filter(
                    Condition::all()
                        .add(product::Column::TenantId.eq(TENANT))
                        .add(product::Column::ProductId.eq(PRODUCT)),
                )
                .exec(&conn)
                .await
                .unwrap_or_else(|e| panic!("walk parent to `{state}`: {e}"));
        }
        insert_sku(
            &conn,
            &scope,
            NewSku {
                sku_id: ORPHAN_CHILD,
                tenant_id: TENANT,
                product_id: PRODUCT,
                sku_code: "FIBRE-500-LIVE".to_owned(),
                region_scope: String::new(),
                brand_scope: String::new(),
                created_by: "principal:author-1".to_owned(),
                created_at: now,
                cloned_from: None,
                cloned_from_version: None,
                sku_type: "product".to_owned(),
                sellable: true,
                plan_tier: "standard".to_owned(),
                tax_category_ref: Some("TC-STD".to_owned()),
                gl_code_ref: Some("GL-4000".to_owned()),
                metering_unit: None,
                usage_type_ref: None,
            },
        )
        .await
        .expect("insert child");
        walk_sku(&conn, &scope, ORPHAN_CHILD, now, &["published"]).await;

        let subject = GateSubject::entity_publish(
            EntityRef {
                tenant_id: TENANT,
                entity_kind: bss_products_sdk::models::EntityKind::Product,
                entity_id: PRODUCT,
            },
            InternalRevision::new(1),
        );
        let policy = MaterialityPolicy::default();
        let evaluator = MaterialityEvaluator::new(Resolution::Resolved(&policy));
        let act = MaterialAct::PolicyMutation;
        submit_approval(
            &conn,
            &scope,
            NewApproval {
                approval_id: ApprovalId::new(ORPHAN_APPROVAL),
                subject: &subject,
                internal_revision: 3,
                content_snapshot: "{}",
                diff_basis: None,
                act: &act,
                evaluator,
                finance_material: false,
                approver_count: 2,
                submitter: ACTOR,
                author_override_ack: None,
                override_conditions: Vec::new(),
            },
            now,
        )
        .await
        .expect("submit");
        approval::Entity::update_many()
            .secure()
            .scope_with(&scope)
            .col_expr(approval::Column::State, Expr::value("satisfied".to_owned()))
            .filter(
                Condition::all()
                    .add(approval::Column::TenantId.eq(TENANT))
                    .add(approval::Column::ApprovalId.eq(ORPHAN_APPROVAL)),
            )
            .exec(&conn)
            .await
            .expect("satisfy");
        consume_approval(&conn, &scope, TENANT, ApprovalId::new(ORPHAN_APPROVAL), now)
            .await
            .expect("consume");
        insert_scheduled_transition(
            &conn,
            &scope,
            &NewScheduledTransition {
                transition_id: ORPHAN_TRANSITION,
                tenant_id: TENANT,
                entity_kind: "product".to_owned(),
                entity_id: PRODUCT,
                kind: "retire".to_owned(),
                at: now - chrono::Duration::hours(1),
                approval_ref: ORPHAN_APPROVAL,
                retirement_reason: Some("cascade".to_owned()),
                now,
            },
        )
        .await
        .expect("insert the pin");
    }

    let ctx = context(&harness);
    sweep(&ctx, ACTOR, now, &CancellationToken::new())
        .await
        .expect("sweep");

    let conn = ctx.db.conn().expect("reopen");
    let row = find_scheduled_transition(&conn, &scope, TENANT, ORPHAN_TRANSITION)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(row.state, "deferred");
    assert_eq!(
        row.outcome_reason.as_deref(),
        Some(RetentionHold::REASON),
        "no-orphan is a deferral, not a wire refusal"
    );
    let parent = find_product(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("find product")
        .expect("product");
    assert_eq!(
        parent.lifecycle_state,
        bss_products_sdk::models::LifecycleState::Deprecated,
        "the flip must not proceed while a published child remains"
    );
}

const HELD_CHILD: Uuid = Uuid::from_u128(0xdd_51);
const HELD_TRANSITION: Uuid = Uuid::from_u128(0xaa_41);
const HELD_APPROVAL: Uuid = Uuid::from_u128(0x00bb_0042);

/// A deprecated (not published) child is not an orphan, but the parent's
/// flip guard is all children `retired`/`discarded`.
#[tokio::test]
async fn a_product_retire_defers_while_a_deprecated_child_is_non_terminal() {
    let harness = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    let now = Utc.with_ymd_and_hms(2026, 9, 4, 12, 0, 0).unwrap();
    {
        let conn = harness.db.conn().expect("scoped connection");
        insert_product(
            &conn,
            &scope,
            NewProduct {
                product_id: PRODUCT,
                tenant_id: TENANT,
                brand_id: BRAND,
                name: "Fibre 500".to_owned(),
                name_normalized: "fibre 500".to_owned(),
                product_code: Some("FIBRE-500".to_owned()),
                region_scope: String::new(),
                brand_scope: String::new(),
                created_by: "principal:author-1".to_owned(),
                created_at: now,
                cloned_from: None,
                cloned_from_version: None,
            },
        )
        .await
        .expect("insert the parent");
        for (step, state) in ["published", "deprecated"].iter().enumerate() {
            let next = 1 + i64::try_from(step).expect("step") + 1;
            product::Entity::update_many()
                .secure()
                .scope_with(&scope)
                .col_expr(product::Column::LifecycleState, Expr::value(*state))
                .col_expr(product::Column::InternalRevision, Expr::value(next))
                .col_expr(product::Column::UpdatedAt, Expr::value(now))
                .filter(
                    Condition::all()
                        .add(product::Column::TenantId.eq(TENANT))
                        .add(product::Column::ProductId.eq(PRODUCT)),
                )
                .exec(&conn)
                .await
                .unwrap_or_else(|e| panic!("walk parent to `{state}`: {e}"));
        }
        insert_sku(
            &conn,
            &scope,
            NewSku {
                sku_id: HELD_CHILD,
                tenant_id: TENANT,
                product_id: PRODUCT,
                sku_code: "FIBRE-500-HELD".to_owned(),
                region_scope: String::new(),
                brand_scope: String::new(),
                created_by: "principal:author-1".to_owned(),
                created_at: now,
                cloned_from: None,
                cloned_from_version: None,
                sku_type: "product".to_owned(),
                sellable: true,
                plan_tier: "standard".to_owned(),
                tax_category_ref: Some("TC-STD".to_owned()),
                gl_code_ref: Some("GL-4000".to_owned()),
                metering_unit: None,
                usage_type_ref: None,
            },
        )
        .await
        .expect("insert child");
        walk_sku(&conn, &scope, HELD_CHILD, now, &["published", "deprecated"]).await;

        let subject = GateSubject::entity_publish(
            EntityRef {
                tenant_id: TENANT,
                entity_kind: bss_products_sdk::models::EntityKind::Product,
                entity_id: PRODUCT,
            },
            InternalRevision::new(1),
        );
        let policy = MaterialityPolicy::default();
        let evaluator = MaterialityEvaluator::new(Resolution::Resolved(&policy));
        let act = MaterialAct::PolicyMutation;
        submit_approval(
            &conn,
            &scope,
            NewApproval {
                approval_id: ApprovalId::new(HELD_APPROVAL),
                subject: &subject,
                internal_revision: 3,
                content_snapshot: "{}",
                diff_basis: None,
                act: &act,
                evaluator,
                finance_material: false,
                approver_count: 2,
                submitter: ACTOR,
                author_override_ack: None,
                override_conditions: Vec::new(),
            },
            now,
        )
        .await
        .expect("submit");
        approval::Entity::update_many()
            .secure()
            .scope_with(&scope)
            .col_expr(approval::Column::State, Expr::value("satisfied".to_owned()))
            .filter(
                Condition::all()
                    .add(approval::Column::TenantId.eq(TENANT))
                    .add(approval::Column::ApprovalId.eq(HELD_APPROVAL)),
            )
            .exec(&conn)
            .await
            .expect("satisfy");
        consume_approval(&conn, &scope, TENANT, ApprovalId::new(HELD_APPROVAL), now)
            .await
            .expect("consume");
        insert_scheduled_transition(
            &conn,
            &scope,
            &NewScheduledTransition {
                transition_id: HELD_TRANSITION,
                tenant_id: TENANT,
                entity_kind: "product".to_owned(),
                entity_id: PRODUCT,
                kind: "retire".to_owned(),
                at: now - chrono::Duration::hours(1),
                approval_ref: HELD_APPROVAL,
                retirement_reason: Some("cascade".to_owned()),
                now,
            },
        )
        .await
        .expect("insert the pin");
    }

    let ctx = context(&harness);
    sweep(&ctx, ACTOR, now, &CancellationToken::new())
        .await
        .expect("sweep");

    let conn = ctx.db.conn().expect("reopen");
    let row = find_scheduled_transition(&conn, &scope, TENANT, HELD_TRANSITION)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(row.state, "deferred");
    assert_eq!(
        row.outcome_reason.as_deref(),
        Some(crate::domain::cascade::PARENT_FLIP_HELD_REASON),
        "parent flip waits on a non-terminal child"
    );
    let parent = find_product(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("find product")
        .expect("product");
    assert_eq!(
        parent.lifecycle_state,
        bss_products_sdk::models::LifecycleState::Deprecated,
        "the flip must not proceed while a deprecated child remains"
    );
}

const POINTER_SKU: Uuid = Uuid::from_u128(0xdd_61);
const BROKEN_TRANSITION: Uuid = Uuid::from_u128(0xaa_51);
const BROKEN_APPROVAL: Uuid = Uuid::from_u128(0x00bb_0052);

#[tokio::test]
async fn a_sku_retire_defers_when_a_live_pointer_names_it() {
    let harness = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    let now = Utc.with_ymd_and_hms(2026, 9, 4, 12, 0, 0).unwrap();
    {
        let conn = harness.db.conn().expect("scoped connection");
        insert_product(
            &conn,
            &scope,
            NewProduct {
                product_id: PRODUCT,
                tenant_id: TENANT,
                brand_id: BRAND,
                name: "Fibre 500".to_owned(),
                name_normalized: "fibre 500".to_owned(),
                product_code: Some("FIBRE-500".to_owned()),
                region_scope: String::new(),
                brand_scope: String::new(),
                created_by: "principal:author-1".to_owned(),
                created_at: now,
                cloned_from: None,
                cloned_from_version: None,
            },
        )
        .await
        .expect("insert the parent");
        for (step, state) in ["published"].iter().enumerate() {
            let next = 1 + i64::try_from(step).expect("step") + 1;
            product::Entity::update_many()
                .secure()
                .scope_with(&scope)
                .col_expr(product::Column::LifecycleState, Expr::value(*state))
                .col_expr(product::Column::InternalRevision, Expr::value(next))
                .col_expr(product::Column::UpdatedAt, Expr::value(now))
                .filter(
                    Condition::all()
                        .add(product::Column::TenantId.eq(TENANT))
                        .add(product::Column::ProductId.eq(PRODUCT)),
                )
                .exec(&conn)
                .await
                .unwrap_or_else(|e| panic!("walk parent to `{state}`: {e}"));
        }
        insert_sku(
            &conn,
            &scope,
            NewSku {
                sku_id: RETIRE_SKU,
                tenant_id: TENANT,
                product_id: PRODUCT,
                sku_code: "FIBRE-500-TGT".to_owned(),
                region_scope: String::new(),
                brand_scope: String::new(),
                created_by: "principal:author-1".to_owned(),
                created_at: now,
                cloned_from: None,
                cloned_from_version: None,
                sku_type: "product".to_owned(),
                sellable: true,
                plan_tier: "standard".to_owned(),
                tax_category_ref: Some("TC-STD".to_owned()),
                gl_code_ref: Some("GL-4000".to_owned()),
                metering_unit: None,
                usage_type_ref: None,
            },
        )
        .await
        .expect("target");
        walk_sku(&conn, &scope, RETIRE_SKU, now, &["published", "deprecated"]).await;
        insert_sku(
            &conn,
            &scope,
            NewSku {
                sku_id: POINTER_SKU,
                tenant_id: TENANT,
                product_id: PRODUCT,
                sku_code: "FIBRE-500-PTR".to_owned(),
                region_scope: String::new(),
                brand_scope: String::new(),
                created_by: "principal:author-1".to_owned(),
                created_at: now,
                cloned_from: None,
                cloned_from_version: None,
                sku_type: "product".to_owned(),
                sellable: true,
                plan_tier: "standard".to_owned(),
                tax_category_ref: Some("TC-STD".to_owned()),
                gl_code_ref: Some("GL-4000".to_owned()),
                metering_unit: None,
                usage_type_ref: None,
            },
        )
        .await
        .expect("pointer");
        walk_sku(
            &conn,
            &scope,
            POINTER_SKU,
            now,
            &["published", "deprecated"],
        )
        .await;
        write_replaced_by(&conn, &scope, TENANT, POINTER_SKU, 3, Some(RETIRE_SKU), now)
            .await
            .expect("point at the target");

        let subject = GateSubject::entity_publish(
            EntityRef {
                tenant_id: TENANT,
                entity_kind: bss_products_sdk::models::EntityKind::Sku,
                entity_id: RETIRE_SKU,
            },
            InternalRevision::new(1),
        );
        let policy = MaterialityPolicy::default();
        let evaluator = MaterialityEvaluator::new(Resolution::Resolved(&policy));
        let act = MaterialAct::PolicyMutation;
        submit_approval(
            &conn,
            &scope,
            NewApproval {
                approval_id: ApprovalId::new(BROKEN_APPROVAL),
                subject: &subject,
                internal_revision: 3,
                content_snapshot: "{}",
                diff_basis: None,
                act: &act,
                evaluator,
                finance_material: false,
                approver_count: 2,
                submitter: ACTOR,
                author_override_ack: None,
                override_conditions: Vec::new(),
            },
            now,
        )
        .await
        .expect("submit");
        approval::Entity::update_many()
            .secure()
            .scope_with(&scope)
            .col_expr(approval::Column::State, Expr::value("satisfied".to_owned()))
            .filter(
                Condition::all()
                    .add(approval::Column::TenantId.eq(TENANT))
                    .add(approval::Column::ApprovalId.eq(BROKEN_APPROVAL)),
            )
            .exec(&conn)
            .await
            .expect("satisfy");
        consume_approval(&conn, &scope, TENANT, ApprovalId::new(BROKEN_APPROVAL), now)
            .await
            .expect("consume");
        insert_scheduled_transition(
            &conn,
            &scope,
            &NewScheduledTransition {
                transition_id: BROKEN_TRANSITION,
                tenant_id: TENANT,
                entity_kind: "sku".to_owned(),
                entity_id: RETIRE_SKU,
                kind: "retire".to_owned(),
                at: now - chrono::Duration::hours(1),
                approval_ref: BROKEN_APPROVAL,
                retirement_reason: Some("replaced".to_owned()),
                now,
            },
        )
        .await
        .expect("insert the pin");
    }

    let ctx = context(&harness);
    sweep(&ctx, ACTOR, now, &CancellationToken::new())
        .await
        .expect("sweep");

    let conn = ctx.db.conn().expect("reopen");
    let row = find_scheduled_transition(&conn, &scope, TENANT, BROKEN_TRANSITION)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(row.state, "deferred");
    assert!(
        row.outcome_reason
            .as_deref()
            .is_some_and(|r| r.starts_with(crate::domain::retirement::REPLACEMENT_CHAIN_BROKEN)),
        "live pointer defers: {:?}",
        row.outcome_reason
    );
}

#[tokio::test]
async fn a_stale_deferral_writes_the_retirement_held_audit() {
    let harness = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    let now = Utc.with_ymd_and_hms(2026, 9, 4, 12, 0, 0).unwrap();
    {
        let conn = harness.db.conn().expect("scoped connection");
        insert_scheduled_transition(
            &conn,
            &scope,
            &NewScheduledTransition {
                transition_id: TRANSITION,
                tenant_id: TENANT,
                entity_kind: "sku".to_owned(),
                entity_id: ENTITY,
                kind: "publish".to_owned(),
                at: now - chrono::Duration::hours(73),
                approval_ref: APPROVAL,
                retirement_reason: None,
                now,
            },
        )
        .await
        .expect("insert");
    }

    let ctx = context(&harness);
    sweep(&ctx, ACTOR, now, &CancellationToken::new())
        .await
        .expect("sweep");

    let conn = ctx.db.conn().expect("reopen");
    let alert = audit_log::Entity::find()
        .secure()
        .scope_with(&scope)
        .filter(Condition::all().add(audit_log::Column::Action.eq("retirement_held")))
        .one(&conn)
        .await
        .expect("query");
    assert!(
        alert.is_some(),
        "a deferral older than retirement_held_alert_hours is recorded"
    );
}

#[tokio::test]
async fn a_sku_retire_defers_when_no_producer_is_registered() {
    let harness = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    let now = Utc.with_ymd_and_hms(2026, 9, 4, 12, 0, 0).unwrap();
    {
        let conn = harness.db.conn().expect("scoped connection");
        insert_product(
            &conn,
            &scope,
            NewProduct {
                product_id: PRODUCT,
                tenant_id: TENANT,
                brand_id: BRAND,
                name: "Fibre 500".to_owned(),
                name_normalized: "fibre 500".to_owned(),
                product_code: Some("FIBRE-500".to_owned()),
                region_scope: String::new(),
                brand_scope: String::new(),
                created_by: "principal:author-1".to_owned(),
                created_at: now,
                cloned_from: None,
                cloned_from_version: None,
            },
        )
        .await
        .expect("insert the parent");
        product::Entity::update_many()
            .secure()
            .scope_with(&scope)
            .col_expr(product::Column::LifecycleState, Expr::value("published"))
            .col_expr(product::Column::InternalRevision, Expr::value(2_i64))
            .col_expr(product::Column::UpdatedAt, Expr::value(now))
            .filter(
                Condition::all()
                    .add(product::Column::TenantId.eq(TENANT))
                    .add(product::Column::ProductId.eq(PRODUCT)),
            )
            .exec(&conn)
            .await
            .expect("parent published");
        insert_sku(
            &conn,
            &scope,
            NewSku {
                sku_id: RETIRE_SKU,
                tenant_id: TENANT,
                product_id: PRODUCT,
                sku_code: "FIBRE-500-R".to_owned(),
                region_scope: String::new(),
                brand_scope: String::new(),
                created_by: "principal:author-1".to_owned(),
                created_at: now,
                cloned_from: None,
                cloned_from_version: None,
                sku_type: "product".to_owned(),
                sellable: true,
                plan_tier: "standard".to_owned(),
                tax_category_ref: Some("TC-STD".to_owned()),
                gl_code_ref: Some("GL-4000".to_owned()),
                metering_unit: None,
                usage_type_ref: None,
            },
        )
        .await
        .expect("insert sku");
        walk_sku(&conn, &scope, RETIRE_SKU, now, &["published", "deprecated"]).await;
        let subject = GateSubject::entity_publish(
            EntityRef {
                tenant_id: TENANT,
                entity_kind: bss_products_sdk::models::EntityKind::Sku,
                entity_id: RETIRE_SKU,
            },
            InternalRevision::new(1),
        );
        let policy = MaterialityPolicy::default();
        let evaluator = MaterialityEvaluator::new(Resolution::Resolved(&policy));
        let act = MaterialAct::PolicyMutation;
        submit_approval(
            &conn,
            &scope,
            NewApproval {
                approval_id: ApprovalId::new(RETIRE_APPROVAL),
                subject: &subject,
                internal_revision: 3,
                content_snapshot: "{}",
                diff_basis: None,
                act: &act,
                evaluator,
                finance_material: false,
                approver_count: 2,
                submitter: ACTOR,
                author_override_ack: None,
                override_conditions: Vec::new(),
            },
            now,
        )
        .await
        .expect("submit");
        approval::Entity::update_many()
            .secure()
            .scope_with(&scope)
            .col_expr(approval::Column::State, Expr::value("satisfied".to_owned()))
            .filter(
                Condition::all()
                    .add(approval::Column::TenantId.eq(TENANT))
                    .add(approval::Column::ApprovalId.eq(RETIRE_APPROVAL)),
            )
            .exec(&conn)
            .await
            .expect("satisfy");
        consume_approval(&conn, &scope, TENANT, ApprovalId::new(RETIRE_APPROVAL), now)
            .await
            .expect("consume");
        insert_scheduled_transition(
            &conn,
            &scope,
            &NewScheduledTransition {
                transition_id: RETIRE_TRANSITION,
                tenant_id: TENANT,
                entity_kind: "sku".to_owned(),
                entity_id: RETIRE_SKU,
                kind: "retire".to_owned(),
                at: now - chrono::Duration::hours(1),
                approval_ref: RETIRE_APPROVAL,
                retirement_reason: Some("end of life".to_owned()),
                now,
            },
        )
        .await
        .expect("insert the pin");
    }

    let ctx = context(&harness);
    sweep(&ctx, ACTOR, now, &CancellationToken::new())
        .await
        .expect("sweep");

    let conn = ctx.db.conn().expect("reopen");
    let row = find_scheduled_transition(&conn, &scope, TENANT, RETIRE_TRANSITION)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(row.state, "deferred");
    assert_eq!(
        row.outcome_reason.as_deref(),
        Some("flip guard: no producers"),
        "an empty registry is NoProducers, not FreshZero"
    );
    let deferred = audit_log::Entity::find()
        .secure()
        .scope_with(&scope)
        .filter(Condition::all().add(audit_log::Column::Action.eq("activation.deferred")))
        .one(&conn)
        .await
        .expect("query");
    assert!(
        deferred.is_some(),
        "a deferral is recorded, not inferred from state"
    );
}

const SKIP_CHILD: Uuid = Uuid::from_u128(0xdd_71);
const SKIP_TRANSITION: Uuid = Uuid::from_u128(0xaa_61);
const SKIP_APPROVAL: Uuid = Uuid::from_u128(0x00bb_0062);

/// P-D-137: a Product flip does not consult `evaluate_reference`. An empty
/// registry is `NoProducers` for a SKU; the same registry must not hold
/// the parent whose children are already terminal.
#[allow(clippy::too_many_lines)] // one scenario, three seeded records, read end to end
#[tokio::test]
async fn a_product_retire_skips_the_07_predicate_when_no_producer_is_registered() {
    let harness = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    let now = Utc.with_ymd_and_hms(2026, 9, 4, 12, 0, 0).unwrap();
    {
        let conn = harness.db.conn().expect("scoped connection");
        insert_product(
            &conn,
            &scope,
            NewProduct {
                product_id: PRODUCT,
                tenant_id: TENANT,
                brand_id: BRAND,
                name: "Fibre 500".to_owned(),
                name_normalized: "fibre 500".to_owned(),
                product_code: Some("FIBRE-500".to_owned()),
                region_scope: String::new(),
                brand_scope: String::new(),
                created_by: "principal:author-1".to_owned(),
                created_at: now,
                cloned_from: None,
                cloned_from_version: None,
            },
        )
        .await
        .expect("insert the parent");
        for (step, state) in ["published", "deprecated"].iter().enumerate() {
            let next = 1 + i64::try_from(step).expect("step") + 1;
            product::Entity::update_many()
                .secure()
                .scope_with(&scope)
                .col_expr(product::Column::LifecycleState, Expr::value(*state))
                .col_expr(product::Column::InternalRevision, Expr::value(next))
                .col_expr(product::Column::UpdatedAt, Expr::value(now))
                .filter(
                    Condition::all()
                        .add(product::Column::TenantId.eq(TENANT))
                        .add(product::Column::ProductId.eq(PRODUCT)),
                )
                .exec(&conn)
                .await
                .unwrap_or_else(|e| panic!("walk parent to `{state}`: {e}"));
        }
        insert_sku(
            &conn,
            &scope,
            NewSku {
                sku_id: SKIP_CHILD,
                tenant_id: TENANT,
                product_id: PRODUCT,
                sku_code: "FIBRE-500-DONE".to_owned(),
                region_scope: String::new(),
                brand_scope: String::new(),
                created_by: "principal:author-1".to_owned(),
                created_at: now,
                cloned_from: None,
                cloned_from_version: None,
                sku_type: "product".to_owned(),
                sellable: true,
                plan_tier: "standard".to_owned(),
                tax_category_ref: Some("TC-STD".to_owned()),
                gl_code_ref: Some("GL-4000".to_owned()),
                metering_unit: None,
                usage_type_ref: None,
            },
        )
        .await
        .expect("insert child");
        walk_sku(
            &conn,
            &scope,
            SKIP_CHILD,
            now,
            &["published", "deprecated", "retired"],
        )
        .await;

        let subject = GateSubject::entity_publish(
            EntityRef {
                tenant_id: TENANT,
                entity_kind: bss_products_sdk::models::EntityKind::Product,
                entity_id: PRODUCT,
            },
            InternalRevision::new(1),
        );
        let policy = MaterialityPolicy::default();
        let evaluator = MaterialityEvaluator::new(Resolution::Resolved(&policy));
        let act = MaterialAct::PolicyMutation;
        submit_approval(
            &conn,
            &scope,
            NewApproval {
                approval_id: ApprovalId::new(SKIP_APPROVAL),
                subject: &subject,
                internal_revision: 3,
                content_snapshot: "{}",
                diff_basis: None,
                act: &act,
                evaluator,
                finance_material: false,
                approver_count: 2,
                submitter: ACTOR,
                author_override_ack: None,
                override_conditions: Vec::new(),
            },
            now,
        )
        .await
        .expect("submit");
        approval::Entity::update_many()
            .secure()
            .scope_with(&scope)
            .col_expr(approval::Column::State, Expr::value("satisfied".to_owned()))
            .filter(
                Condition::all()
                    .add(approval::Column::TenantId.eq(TENANT))
                    .add(approval::Column::ApprovalId.eq(SKIP_APPROVAL)),
            )
            .exec(&conn)
            .await
            .expect("satisfy");
        consume_approval(&conn, &scope, TENANT, ApprovalId::new(SKIP_APPROVAL), now)
            .await
            .expect("consume");
        insert_scheduled_transition(
            &conn,
            &scope,
            &NewScheduledTransition {
                transition_id: SKIP_TRANSITION,
                tenant_id: TENANT,
                entity_kind: "product".to_owned(),
                entity_id: PRODUCT,
                kind: "retire".to_owned(),
                at: now - chrono::Duration::hours(1),
                approval_ref: SKIP_APPROVAL,
                retirement_reason: Some("cascade".to_owned()),
                now,
            },
        )
        .await
        .expect("insert the pin");
    }

    let ctx = context(&harness);
    sweep(&ctx, ACTOR, now, &CancellationToken::new())
        .await
        .expect("sweep");

    let conn = ctx.db.conn().expect("reopen");
    let row = find_scheduled_transition(&conn, &scope, TENANT, SKIP_TRANSITION)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(
        row.state, "applied",
        "the Product flip skips 07; an empty registry must not hold it: {:?}",
        row.outcome_reason
    );
    let parent = find_product(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("find product")
        .expect("product");
    assert_eq!(
        parent.lifecycle_state,
        bss_products_sdk::models::LifecycleState::Retired,
        "children terminal is the Product guard (P-D-115), not evaluate_reference"
    );
}

/// `dod-classification-errors`' scheduled-lane clause: `USAGE_TYPE_UNAVAILABLE`
/// is the one publish refusal this lane defers (transient dependency, under
/// the attempt budget); a decision about the SKU fails the run.
#[test]
fn only_an_unavailable_collector_is_a_transient_publish_refusal() {
    use crate::domain::activation::{AttemptBudget, DeferralPopulation, DoorRefusal, RunFinish};
    use crate::infra::activation_runner::publish_refusal_is_transient;

    assert!(publish_refusal_is_transient("USAGE_TYPE_UNAVAILABLE"));
    for decided in [
        "USAGE_TYPE_UNRESOLVED",
        "ACCOUNTING_CODE_REQUIRED",
        "BUNDLE_OVERRIDE_REQUIRED",
        "STALE_REVISION",
    ] {
        assert!(!publish_refusal_is_transient(decided), "{decided}");
    }
    let budget = AttemptBudget { max: 5 };
    let finish = crate::domain::activation::classify_door_refusal(
        DoorRefusal {
            code: "USAGE_TYPE_UNAVAILABLE",
            transient: publish_refusal_is_transient("USAGE_TYPE_UNAVAILABLE"),
        },
        1,
        budget,
    )
    .expect("a transient code is not a stale-approval refusal");
    assert!(
        matches!(
            finish,
            RunFinish::Deferred {
                population: DeferralPopulation::TransientDependency,
                ..
            }
        ),
        "the collector's silence joins the deferred set, not the failed one: {finish:?}"
    );
}
