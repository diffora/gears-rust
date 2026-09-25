//! Phase 3 tables on Postgres: every new migration applies, re-applies and reverses on its own,
//! and every new table keeps its keys and CHECKs through the same scoped repositories.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod pg_support;
use bss_approval::{Store, Unit, UnitState};
use bss_pricing::{
    domain::plan,
    infra::storage::{
        RepoError,
        entity::{plan as plan_e, plan_item, plan_revision, price_book, price_book_entry},
        migrations::Migrator,
        repo::{
            approval_repo::PricingApprovalStore, book_repo, plan_item_repo, plan_repo,
            plan_revision_repo, price_book_entry_repo, price_repo,
        },
    },
};
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement};
use sea_orm_migration::{MigratorTrait, SchemaManager};
use toolkit_db::secure::AccessScope;
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

fn now() -> time::OffsetDateTime {
    // Postgres keeps microseconds; a fixture time with nanoseconds would not read back equal.
    time::OffsetDateTime::from_unix_timestamp(1_790_000_000).unwrap()
}
fn date(s: &str) -> time::Date {
    time::Date::parse(s, &time::format_description::well_known::Iso8601::DATE).unwrap()
}
fn conflict<T: std::fmt::Debug>(result: Result<T, RepoError>, code: &str) {
    match result {
        Err(RepoError::Conflict { code: c }) if c == code => {}
        other => panic!("expected {code}, got {other:?}"),
    }
}

async fn bss_table(conn: &DatabaseConnection, table: &str) -> bool {
    let row = conn
        .query_one_raw(Statement::from_string(
            DbBackend::Postgres,
            format!(
                "SELECT count(*)::bigint AS n FROM information_schema.tables \
                 WHERE table_schema = 'bss' AND table_name = '{table}'"
            ),
        ))
        .await
        .unwrap()
        .unwrap();
    row.try_get::<i64>("", "n").unwrap() == 1
}

/// The chain up to `index`, then the step twice up and twice down, on a fresh database.
async fn migration(index: usize, tables: &[&str]) {
    let pg = pg_support::Pg::empty().await;
    let conn = Database::connect(pg.url(true)).await.unwrap();
    let manager = SchemaManager::new(&conn);
    let chain = Migrator::migrations();
    for prior in &chain[..index] {
        prior.up(&manager).await.unwrap();
    }
    let step = &chain[index];
    step.up(&manager).await.unwrap();
    step.up(&manager).await.unwrap();
    for table in tables {
        assert!(bss_table(&conn, table).await, "{table} after up");
    }
    step.down(&manager).await.unwrap();
    step.down(&manager).await.unwrap();
    for table in tables {
        assert!(!bss_table(&conn, table).await, "{table} after down");
    }
}

#[tokio::test]
#[ignore = "needs the Postgres harness"]
async fn postgres_m20260926_000010_plan() {
    migration(10, &["pricing_plan"]).await;
}
#[tokio::test]
#[ignore = "needs the Postgres harness"]
async fn postgres_m20260926_000011_plan_revision() {
    migration(11, &["pricing_plan_revision"]).await;
}
#[tokio::test]
#[ignore = "needs the Postgres harness"]
async fn postgres_m20260926_000012_plan_item() {
    migration(12, &["pricing_plan_item"]).await;
}

struct Seed {
    provider: DBProvider<DbError>,
    scope: AccessScope,
    tenant: Uuid,
    book: price_book::Model,
    entry: price_book_entry::Model,
    plan: plan_e::Model,
    revision: plan_revision::Model,
}
async fn seed() -> Seed {
    let pg = pg_support::Pg::applied().await;
    let provider = DBProvider::<DbError>::new(pg.db().await);
    let conn = provider.conn().unwrap();
    let tenant = Uuid::new_v4();
    let scope = AccessScope::for_tenant(tenant);
    let book = book_repo::insert(
        &conn,
        &scope,
        price_book::Model {
            id: Uuid::new_v4(),
            tenant_id: tenant,
            code: "eur".into(),
            name: "Default EUR".into(),
            currency: "EUR".into(),
            valid_from: None,
            valid_until: None,
            version: 1,
            created_at: now(),
            updated_at: now(),
        },
    )
    .await
    .unwrap();
    let entry = price_book_entry_repo::insert(
        &conn,
        &scope,
        price_book_entry::Model {
            id: Uuid::new_v4(),
            tenant_id: tenant,
            book_id: book.id,
            sku_id: Uuid::new_v4(),
            charge_kind: "usage".into(),
            period: None,
            dimension_key: None,
            invoice_line_override: None,
            reservation_id: Uuid::new_v4(),
            reference_state: "confirmed".into(),
            version: 1,
            created_at: now(),
            updated_at: now(),
        },
    )
    .await
    .unwrap();
    let plan = plan_repo::insert(&conn, &scope, plan(tenant, "pro"))
        .await
        .unwrap();
    let revision = plan_revision_repo::insert(&conn, &scope, revision(&plan, &book, 1))
        .await
        .unwrap();
    Seed {
        provider,
        scope,
        tenant,
        book,
        entry,
        plan,
        revision,
    }
}
fn plan(tenant: Uuid, code: &str) -> plan_e::Model {
    plan_e::Model {
        id: Uuid::new_v4(),
        tenant_id: tenant,
        code: code.into(),
        name: "Pro".into(),
        published_rev: None,
        version: 1,
        created_by: Uuid::new_v4(),
        created_at: now(),
        updated_at: now(),
    }
}
fn revision(p: &plan_e::Model, b: &price_book::Model, rev_no: i32) -> plan_revision::Model {
    plan_revision::Model {
        id: Uuid::new_v4(),
        tenant_id: p.tenant_id,
        plan_id: p.id,
        rev_no,
        book_id: b.id,
        state: "draft".into(),
        available_from: Some(date("2026-10-01")),
        pending_unit_id: None,
        approved_by_unit_id: None,
        published_at: None,
        version: 1,
        created_by: Uuid::new_v4(),
        created_at: now(),
        updated_at: now(),
    }
}
async fn unit(s: &Seed, kind: &str) -> Uuid {
    let id = Uuid::new_v4();
    let (scope, tenant, kind) = (s.scope.clone(), s.tenant, kind.to_owned());
    price_repo::transaction(&s.provider.db(), move |tx| {
        let scope = scope.clone();
        let kind = kind.clone();
        Box::pin(async move {
            PricingApprovalStore {
                scope,
                tenant_id: tenant,
            }
            .insert_unit(
                tx,
                &Unit {
                    id,
                    tenant_id: tenant,
                    kind,
                    ref_type: "plan_revision".into(),
                    ref_id: Uuid::new_v4(),
                    state: UnitState::Pending,
                    common_effective_date: None,
                    quorum_required: 1,
                    generation: 1,
                    submitted_by: Uuid::new_v4(),
                    submitted_at: now(),
                    decided_at: None,
                    decided_note: None,
                    snapshot: serde_json::json!({}),
                    snapshot_hash: "hash".into(),
                    version: 1,
                },
                &[],
            )
            .await
            .map_err(|e| RepoError::Db(e.to_string()))
        })
    })
    .await
    .unwrap();
    id
}

#[tokio::test]
#[ignore = "needs the Postgres harness"]
async fn postgres_pricing_plan_keys_and_projection() {
    let s = seed().await;
    let conn = s.provider.conn().unwrap();
    assert_eq!(
        plan_repo::find(&conn, &s.scope, s.tenant, s.plan.id)
            .await
            .unwrap(),
        Some(s.plan.clone())
    );
    conflict(
        plan_repo::insert(&conn, &s.scope, plan(s.tenant, "pro")).await,
        "PLAN_CODE_TAKEN",
    );
    plan_repo::set_published(&conn, &s.scope, s.tenant, s.plan.id, 1, 1, now())
        .await
        .unwrap();
    conflict(
        plan_repo::rename(
            &conn,
            &s.scope,
            s.tenant,
            s.plan.id,
            1,
            "Late".into(),
            now(),
        )
        .await,
        "STALE_REVISION",
    );
    let got = plan_repo::find(&conn, &s.scope, s.tenant, s.plan.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((got.published_rev, got.version), (Some(1), 2));
    // Another tenant may reuse the code.
    let foreign = Uuid::new_v4();
    plan_repo::insert(
        &conn,
        &AccessScope::for_tenant(foreign),
        plan(foreign, "pro"),
    )
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "needs the Postgres harness"]
async fn postgres_pricing_plan_revision_partial_indexes_and_lifecycle() {
    let s = seed().await;
    let conn = s.provider.conn().unwrap();
    assert_eq!(
        plan_revision_repo::find(&conn, &s.scope, s.tenant, s.revision.id)
            .await
            .unwrap(),
        Some(s.revision.clone())
    );
    // A superseded rev 1 joins no partial index, so only the rev_no key can refuse it.
    let mut same_no = revision(&s.plan, &s.book, 1);
    same_no.state = "superseded".into();
    conflict(
        plan_revision_repo::insert(&conn, &s.scope, same_no).await,
        "REVISION_NO_TAKEN",
    );
    conflict(
        plan_revision_repo::insert(&conn, &s.scope, revision(&s.plan, &s.book, 2)).await,
        "REVISION_DRAFT_EXISTS",
    );
    let u = unit(&s, "plan_revision").await;
    assert!(
        plan_revision_repo::try_lock(&conn, &s.scope, s.tenant, s.revision.id, u, 1)
            .await
            .unwrap()
    );
    plan_revision_repo::publish(&conn, &s.scope, s.tenant, s.revision.id, u, now())
        .await
        .unwrap();
    let mut second = revision(&s.plan, &s.book, 2);
    second.state = "published".into();
    conflict(
        plan_revision_repo::insert(&conn, &s.scope, second).await,
        "REVISION_PUBLISHED_EXISTS",
    );
    let next = plan_revision_repo::insert(&conn, &s.scope, revision(&s.plan, &s.book, 2))
        .await
        .unwrap();
    let u2 = unit(&s, "plan_revision").await;
    assert!(
        plan_revision_repo::try_lock(&conn, &s.scope, s.tenant, next.id, u2, 1)
            .await
            .unwrap()
    );
    conflict(
        plan_revision_repo::publish(&conn, &s.scope, s.tenant, next.id, u2, now()).await,
        "REVISION_PUBLISHED_EXISTS",
    );
    plan_revision_repo::supersede(&conn, &s.scope, s.tenant, s.revision.id, 3, now())
        .await
        .unwrap();
    plan_revision_repo::publish(&conn, &s.scope, s.tenant, next.id, u2, now())
        .await
        .unwrap();
    let mut odd = revision(&s.plan, &s.book, 3);
    odd.state = "retired".into();
    assert!(
        plan_revision_repo::insert(&conn, &s.scope, odd)
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "needs the Postgres harness"]
async fn postgres_pricing_plan_item_keys_checks_and_reference() {
    let s = seed().await;
    let conn = s.provider.conn().unwrap();
    let item = plan_item::Model {
        id: Uuid::new_v4(),
        tenant_id: s.tenant,
        revision_id: s.revision.id,
        sku_id: s.entry.sku_id,
        price_book_entry_id: Some(s.entry.id),
        treatment: "included".into(),
        included_qty: Some("12345678901234567.89".into()),
        qty_min: None,
        reservation_id: None,
        reference_state: "unreserved".into(),
        version: 1,
        created_by: Uuid::new_v4(),
        created_at: now(),
        updated_at: now(),
    };
    assert_eq!(
        plan_item_repo::insert(&conn, &s.scope, item.clone())
            .await
            .unwrap(),
        item,
        "the quantity reads back as exact text"
    );
    conflict(
        plan_item_repo::insert(
            &conn,
            &s.scope,
            plan_item::Model {
                id: Uuid::new_v4(),
                ..item.clone()
            },
        )
        .await,
        "ITEM_SKU_TAKEN",
    );
    for (what, m) in [
        (
            "paid without an entry",
            plan_item::Model {
                id: Uuid::new_v4(),
                sku_id: Uuid::new_v4(),
                treatment: "paid".into(),
                price_book_entry_id: None,
                ..item.clone()
            },
        ),
        (
            "signed quantity",
            // No entry: an entry of the base item's SKU would refuse a new SKU on its own
            // (ITEM_ENTRY_SKU_MISMATCH), before the CHECK under test is reached.
            plan_item::Model {
                id: Uuid::new_v4(),
                sku_id: Uuid::new_v4(),
                price_book_entry_id: None,
                included_qty: Some("-1".into()),
                ..item.clone()
            },
        ),
        (
            "negative minimum",
            // No entry: an entry of the base item's SKU would refuse a new SKU on its own
            // (ITEM_ENTRY_SKU_MISMATCH), before the CHECK under test is reached.
            plan_item::Model {
                id: Uuid::new_v4(),
                sku_id: Uuid::new_v4(),
                price_book_entry_id: None,
                qty_min: Some(-1),
                ..item.clone()
            },
        ),
    ] {
        assert!(
            plan_item_repo::insert(&conn, &s.scope, m).await.is_err(),
            "{what}"
        );
    }
    let receipt = Uuid::new_v4();
    plan_item_repo::set_reference(
        &conn,
        &s.scope,
        s.tenant,
        item.id,
        1,
        plan::ReferenceState::ConfirmationPending,
        Some(receipt),
        now(),
    )
    .await
    .unwrap();
    conflict(
        plan_item_repo::set_reference(
            &conn,
            &s.scope,
            s.tenant,
            item.id,
            1,
            plan::ReferenceState::Lost,
            Some(receipt),
            now(),
        )
        .await,
        "STALE_REVISION",
    );
    assert!(
        plan_item_repo::names_entry(&conn, &s.scope, s.tenant, s.entry.id)
            .await
            .unwrap()
    );
}
