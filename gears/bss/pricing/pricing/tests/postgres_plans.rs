//! Phase 3 on Postgres. The tables: every new migration applies, re-applies and reverses on its
//! own, and every new table keeps its keys and CHECKs through the same scoped repositories. The
//! doors (run 3.5): an item add racing the submit that applies its revision on two pools is
//! ordered by SSI (refused `REVISION_NOT_DRAFT`, or carried by the unit), two submits of one
//! revision make one unit, and a plan item's create and a copied item's attach round-trip through
//! the Products registry double (reserve, write, confirm; attach, confirm).
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod pg_support;
mod plan_support;
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
    migration(11, &["pricing_plan"]).await;
}
#[tokio::test]
#[ignore = "needs the Postgres harness"]
async fn postgres_m20260926_000011_plan_revision() {
    migration(12, &["pricing_plan_revision"]).await;
}
#[tokio::test]
#[ignore = "needs the Postgres harness"]
async fn postgres_m20260926_000012_plan_item() {
    migration(13, &["pricing_plan_item"]).await;
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

// ------------------------------------------------------------------ the doors on Postgres (run 3.5)

use bss_products_sdk::{
    ReferenceRegistryV1,
    models::{ReferenceKind, ReferenceState, ReservationReceipt, Sku, SkuType, SkuVersion},
};
use plan_support::{
    Catalog, Fixture,
    entry_support::{app_for, state_on, user_of},
    id_of, scope,
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;

/// One point a door passes through the registry: armed, the next call parks until the test
/// resumes it, so a race is driven into one interleaving on purpose.
#[derive(Default)]
struct Gate {
    armed: AtomicBool,
    parked: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}
impl Gate {
    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }
    async fn pass(&self) {
        if self.armed.swap(false, Ordering::SeqCst) {
            self.parked.notify_one();
            self.resume.notified().await;
        }
    }
}
/// The catalog double of one pricing process, with a gate on its reserves and one on its SKU
/// reads; every process shares the catalog's SKUs and reservations.
struct Gated {
    catalog: Arc<Catalog>,
    reserve: Gate,
    read: Gate,
}
#[async_trait::async_trait]
impl ReferenceRegistryV1 for Gated {
    async fn reserve(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        sku: Uuid,
        kind: ReferenceKind,
        ref_id: Uuid,
    ) -> Result<ReservationReceipt, CanonicalError> {
        self.reserve.pass().await;
        self.catalog.reserve(ctx, tenant, sku, kind, ref_id).await
    }
    async fn confirm(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        id: Uuid,
    ) -> Result<(), CanonicalError> {
        self.catalog.confirm(ctx, tenant, id).await
    }
    async fn release(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        id: Uuid,
    ) -> Result<(), CanonicalError> {
        self.catalog.release(ctx, tenant, id).await
    }
    async fn states(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        ids: &[Uuid],
    ) -> Result<Vec<(Uuid, ReferenceState)>, CanonicalError> {
        self.catalog.states(ctx, tenant, ids).await
    }
    async fn sku_for_write(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        id: Uuid,
    ) -> Result<Sku, CanonicalError> {
        self.read.pass().await;
        self.catalog.sku_for_write(ctx, tenant, id).await
    }
    async fn sku_version_as_of(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        id: Uuid,
        date: time::Date,
    ) -> Result<Option<SkuVersion>, CanonicalError> {
        self.catalog.sku_version_as_of(ctx, tenant, id, date).await
    }
}

/// Two pricing processes of one tenant on one Postgres database, each on its own pool and its
/// own gated view of one catalog, and one principal who authors and submits.
struct Pools {
    catalog: Arc<Catalog>,
    a: Fixture,
    gate_a: Arc<Gated>,
    b: Fixture,
    gate_b: Arc<Gated>,
}
async fn pool(pg: &pg_support::Pg, registry: Arc<Gated>, ctx: &SecurityContext) -> Fixture {
    let db = DBProvider::<DbError>::new(pg.db().await);
    let state = state_on(db.clone(), registry).await;
    let app = app_for(state.clone(), ctx.subject_tenant_id());
    Fixture {
        dsn: pg.url(true),
        state,
        app: app.clone(),
        denied: app,
        ctx: ctx.clone(),
        db,
    }
}
async fn pools() -> Pools {
    let pg = pg_support::Pg::applied().await;
    let catalog = Arc::new(Catalog::default());
    let ctx = user_of(Uuid::new_v4());
    let gated = || {
        Arc::new(Gated {
            catalog: catalog.clone(),
            reserve: Gate::default(),
            read: Gate::default(),
        })
    };
    let (gate_a, gate_b) = (gated(), gated());
    let a = pool(&pg, gate_a.clone(), &ctx).await;
    let b = pool(&pg, gate_b.clone(), &ctx).await;
    Pools {
        catalog,
        a,
        gate_a,
        b,
        gate_b,
    }
}
/// A usage SKU of the catalog with an entry on `book` priced (approved) from 2020 with an open
/// tail: an item on it is green.
async fn priced(f: &Fixture, catalog: &Catalog, book: Uuid) -> (Uuid, Uuid) {
    let sku = catalog.sku(SkuType::Usage);
    let entry = plan_support::entry(f, book, sku, "usage", None).await;
    let conn = f.db.conn().unwrap();
    let e = price_book_entry_repo::find(&conn, &scope(f), f.ctx.subject_tenant_id(), entry)
        .await
        .unwrap()
        .unwrap();
    let mut p = plan_support::entry_support::price(&e);
    p.state = "approved".into();
    p.effective_from = date("2020-01-01");
    price_repo::insert(&conn, &scope(f), p).await.unwrap();
    (sku, entry)
}
/// A plan through its door whose draft rev 1 holds one green item: `(plan, revision)`.
async fn green_plan(p: &Pools, code: &str) -> (Uuid, Uuid, Uuid) {
    let book = plan_support::book(&p.a, code).await;
    let (plan, revision) = plan_support::plan(&p.a, code, book).await;
    let (sku, entry) = priced(&p.a, &p.catalog, book).await;
    plan_support::item(&p.a, revision, sku, Some(entry), "paid").await;
    (id_of(&plan["id"]), revision, book)
}
async fn quorum(f: &Fixture, quorum: u32) {
    let (_, _, tag) = f
        .call("GET", "/approval-policy", json!({}), None, None)
        .await;
    let (s, b, _) = f
        .call(
            "PUT",
            "/approval-policy",
            json!({"kind":"plan_revision","quorum":quorum}),
            Some(&tag),
            None,
        )
        .await;
    assert_eq!(s, 200, "{b}");
}
fn submit_path(revision: Uuid) -> String {
    format!("/plan-revisions/{revision}/submit")
}
fn add_path(revision: Uuid) -> String {
    format!("/plan-revisions/{revision}/items")
}
async fn read_revision(f: &Fixture, revision: Uuid) -> Value {
    let (s, b, _) = f
        .call(
            "GET",
            &format!("/plan-revisions/{revision}"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 200, "{b}");
    b
}
/// The SKUs of a list of items (a revision's, or a unit's `after`), sorted.
fn skus_of(items: &Value) -> Vec<String> {
    let mut out: Vec<String> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["sku_id"].as_str().unwrap().to_owned())
        .collect();
    out.sort();
    out
}
/// Every reference op not done, as the ticker would see them past the in-flight grace.
async fn due(f: &Fixture) -> Vec<bss_pricing::infra::storage::entity::reference_op::Model> {
    bss_pricing::infra::storage::repo::reference_op_repo::due(
        &f.db.conn().unwrap(),
        &scope(f),
        time::OffsetDateTime::now_utc() + time::Duration::days(2),
        10,
    )
    .await
    .unwrap()
}
/// The stored units of one revision, as the queue lists them.
async fn units_of(f: &Fixture, revision: Uuid) -> Vec<Value> {
    let (s, b, _) = f
        .call(
            "GET",
            &format!("/approval-units?kind=plan_revision&ref_id={revision}"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 200, "{b}");
    b["items"].as_array().unwrap().clone()
}
/// What was published is what the unit fingerprinted: the published revision's items are
/// exactly the unit's `after`, so no item reached a revision after its lock.
async fn published_is_what_was_approved(f: &Fixture, revision: Uuid, receipt: &Value) -> Value {
    let stored = read_revision(f, revision).await;
    assert_eq!(stored["state"], "published", "{stored}");
    assert_eq!(
        skus_of(&stored["items"]),
        skus_of(&receipt["unit"]["snapshot"]["after"]["items"]),
        "the published revision carries exactly the unit's content: {stored} / {receipt}"
    );
    stored
}

/// An add that passed the door while the revision was a draft reaches its write after a submit
/// on another pool locked and published it: Tx B re-reads the revision and refuses
/// `REVISION_NOT_DRAFT`, the reservation is released, and the published revision is exactly the
/// unit's content.
#[tokio::test]
#[ignore = "needs the Postgres harness"]
async fn postgres_an_item_add_reaching_its_write_after_the_lock_is_refused() {
    let p = pools().await;
    let (_, revision, book) = green_plan(&p, "late").await;
    quorum(&p.a, 0).await;
    let (sku, entry) = priced(&p.a, &p.catalog, book).await;
    p.gate_a.reserve.arm();
    let path = add_path(revision);
    let mut add = Box::pin(p.a.call(
        "POST",
        &path,
        json!({"sku_id":sku,"price_book_entry_id":entry,"treatment":"paid"}),
        None,
        Some("add"),
    ));
    tokio::select! {
        result = &mut add => panic!("the add did not reach its reserve: {result:?}"),
        () = p.gate_a.reserve.parked.notified() => {}
    }
    let (s, receipt, _) =
        p.b.call(
            "POST",
            &submit_path(revision),
            json!({}),
            None,
            Some("submit"),
        )
        .await;
    assert_eq!(s, 201, "{receipt}");
    assert_eq!(receipt["applied"], true, "{receipt}");
    p.gate_a.reserve.resume.notify_one();
    let (s, refused, _) = add.await;
    assert_eq!(s, 409, "{refused}");
    assert!(
        refused.to_string().contains("REVISION_NOT_DRAFT"),
        "{refused}"
    );
    let stored = published_is_what_was_approved(&p.a, revision, &receipt).await;
    assert!(
        !skus_of(&stored["items"]).contains(&sku.to_string()),
        "no item was added after the lock: {stored}"
    );
    assert_eq!(p.catalog.reserves(), 1, "the add reserved once");
    assert_eq!(p.catalog.releases(), 1, "and released what it reserved");
}

/// An add that commits while a submit on another pool has read the revision's items but not yet
/// locked it: SSI fails the submit's transaction, its retry reads the new item, and the unit
/// carries it — the published revision is exactly the unit's content.
#[tokio::test]
#[ignore = "needs the Postgres harness"]
async fn postgres_an_item_add_committed_inside_a_submit_is_carried_by_the_unit() {
    let p = pools().await;
    let (_, revision, book) = green_plan(&p, "early").await;
    quorum(&p.a, 0).await;
    let (sku, entry) = priced(&p.a, &p.catalog, book).await;
    p.gate_b.read.arm();
    let path = submit_path(revision);
    let mut submit = Box::pin(p.b.call("POST", &path, json!({}), None, Some("submit")));
    tokio::select! {
        result = &mut submit => panic!("the submit did not reach its SKU read: {result:?}"),
        () = p.gate_b.read.parked.notified() => {}
    }
    let (s, added, _) =
        p.a.call(
            "POST",
            &add_path(revision),
            json!({"sku_id":sku,"price_book_entry_id":entry,"treatment":"paid"}),
            None,
            Some("add"),
        )
        .await;
    assert_eq!(s, 201, "the add lands before the lock: {added}");
    assert_eq!(added["reference_state"], "confirmed", "{added}");
    p.gate_b.read.resume.notify_one();
    let (s, receipt, _) = submit.await;
    assert_eq!(s, 201, "{receipt}");
    assert_eq!(receipt["applied"], true, "{receipt}");
    let stored = published_is_what_was_approved(&p.a, revision, &receipt).await;
    assert!(
        skus_of(&stored["items"]).contains(&sku.to_string()),
        "the unit carries the item that landed before its lock: {stored}"
    );
    assert_eq!(units_of(&p.a, revision).await.len(), 1);
}

/// The same race left to the scheduler, round after round: whichever order the two pools
/// commit in, the add is refused `REVISION_NOT_DRAFT` or carried, and what is published is
/// what the unit fingerprinted.
#[tokio::test]
#[ignore = "needs the Postgres harness"]
async fn postgres_an_item_add_racing_a_publishing_submit_never_lands_after_the_lock() {
    let p = pools().await;
    quorum(&p.a, 0).await;
    for round in 0..6 {
        let (_, revision, book) = green_plan(&p, &format!("race-{round}")).await;
        let (sku, entry) = priced(&p.a, &p.catalog, book).await;
        let add_body = json!({"sku_id":sku,"price_book_entry_id":entry,"treatment":"paid"});
        let (add_key, submit_key) = (format!("add-{round}"), format!("submit-{round}"));
        let (submit_at, add_at) = (submit_path(revision), add_path(revision));
        let submit =
            p.b.call("POST", &submit_at, json!({}), None, Some(&submit_key));
        let add = p.a.call("POST", &add_at, add_body, None, Some(&add_key));
        // Alternate which door is polled first, so both orders get their chance.
        let (submitted, added) = if round % 2 == 0 {
            tokio::join!(submit, add)
        } else {
            let (added, submitted) = tokio::join!(add, submit);
            (submitted, added)
        };
        assert_eq!(submitted.0, 201, "round {round}: {submitted:?}");
        let stored = published_is_what_was_approved(&p.a, revision, &submitted.1).await;
        let carried = skus_of(&stored["items"]).contains(&sku.to_string());
        match added.0 {
            201 => assert!(carried, "round {round}: a landed add is carried: {stored}"),
            409 => {
                assert!(
                    added.1.to_string().contains("REVISION_NOT_DRAFT"),
                    "round {round}: {added:?}"
                );
                assert!(!carried, "round {round}: {stored}");
            }
            other => panic!("round {round}: the add answered {other}: {added:?}"),
        }
    }
}

/// Two submits of one revision on two pools, one parked inside its transaction while the other
/// commits: one unit, and the late submit is refused.
#[tokio::test]
#[ignore = "needs the Postgres harness"]
async fn postgres_two_submits_of_one_revision_make_one_unit() {
    let p = pools().await;
    let (_, revision, _) = green_plan(&p, "twice").await;
    quorum(&p.a, 1).await;
    p.gate_b.read.arm();
    let path = submit_path(revision);
    let mut late = Box::pin(p.b.call("POST", &path, json!({}), None, Some("late")));
    tokio::select! {
        result = &mut late => panic!("the late submit did not reach its SKU read: {result:?}"),
        () = p.gate_b.read.parked.notified() => {}
    }
    let (s, first, _) =
        p.a.call(
            "POST",
            &submit_path(revision),
            json!({}),
            None,
            Some("first"),
        )
        .await;
    assert_eq!(s, 201, "{first}");
    assert_eq!(first["applied"], false, "quorum 1 leaves the unit pending");
    p.gate_b.read.resume.notify_one();
    let (s, refused, _) = late.await;
    assert_eq!(s, 409, "{refused}");
    let text = refused.to_string();
    assert!(
        text.contains("REVISION_NOT_DRAFT") || text.contains("ROW_LOCKED_PENDING"),
        "{refused}"
    );
    let units = units_of(&p.a, revision).await;
    assert_eq!(units.len(), 1, "{units:?}");
    assert_eq!(units[0]["id"], first["unit"]["id"]);
    let stored = read_revision(&p.a, revision).await;
    assert_eq!(stored["state"], "pending");
    assert_eq!(stored["pending_unit_id"], first["unit"]["id"]);

    // And left to the scheduler: still one unit per revision.
    let (_, revision, _) = green_plan(&p, "twice-free").await;
    let path = submit_path(revision);
    let (left, right) = tokio::join!(
        p.a.call("POST", &path, json!({}), None, Some("left")),
        p.b.call("POST", &path, json!({}), None, Some("right")),
    );
    let mut statuses = [left.0, right.0];
    statuses.sort_unstable();
    assert_eq!(statuses, [201, 409], "{left:?} / {right:?}");
    assert_eq!(units_of(&p.a, revision).await.len(), 1);
}

/// A plan item added through its door reserves (kind `plan_item`), is written and confirmed; a
/// copy of the published revision writes its item unreserved and attaches it (reserve, confirm)
/// before it answers; nothing is left for the ticker and nothing is released.
#[tokio::test]
#[ignore = "needs the Postgres harness"]
async fn postgres_an_item_create_and_a_copied_items_attach_round_trip_through_the_registry() {
    let p = pools().await;
    let book = plan_support::book(&p.a, "trip").await;
    let (plan, revision) = plan_support::plan(&p.a, "trip", book).await;
    let plan = id_of(&plan["id"]);
    let (sku, entry) = priced(&p.a, &p.catalog, book).await;
    let body = json!({"sku_id":sku,"price_book_entry_id":entry,"treatment":"paid"});
    let (s, created, _) =
        p.a.call("POST", &add_path(revision), body.clone(), None, Some("add"))
            .await;
    assert_eq!(s, 201, "{created}");
    assert_eq!(created["reference_state"], "confirmed", "{created}");
    let reservation = created["reservation_id"].clone();
    let refs = p.catalog.refs.lock().unwrap().clone();
    assert_eq!(
        refs.get(&id_of(&created["id"])).copied(),
        Some((id_of(&reservation), ReferenceState::Confirmed)),
        "Products holds the item's confirmed reservation"
    );
    assert_eq!(
        *p.catalog.reserve_kinds.lock().unwrap(),
        vec![ReferenceKind::PlanItem]
    );
    let (s, replay, _) =
        p.a.call("POST", &add_path(revision), body, None, Some("add"))
            .await;
    assert_eq!((s, &replay), (201, &created), "the key replays its answer");
    assert!(due(&p.a).await.is_empty(), "the door finished its op");

    quorum(&p.a, 0).await;
    let (s, receipt, _) =
        p.a.call(
            "POST",
            &submit_path(revision),
            json!({}),
            None,
            Some("submit"),
        )
        .await;
    assert_eq!(s, 201, "{receipt}");
    assert_eq!(receipt["revision"]["state"], "published", "{receipt}");
    let (s, copy, _) =
        p.a.call(
            "POST",
            &format!("/plans/{plan}/revisions"),
            json!({}),
            None,
            Some("copy"),
        )
        .await;
    assert_eq!(s, 201, "{copy}");
    assert_eq!(
        copy["items"][0]["reference_state"], "unreserved",
        "the copy answers what its transaction wrote (D-413): {copy}"
    );
    let rev2 = id_of(&copy["id"]);
    let stored = read_revision(&p.a, rev2).await;
    let copied = &stored["items"][0];
    assert_eq!(copied["sku_id"], sku.to_string(), "{stored}");
    assert_eq!(
        copied["reference_state"], "confirmed",
        "the attach reserved and confirmed: {stored}"
    );
    assert_ne!(copied["reservation_id"], reservation, "its own reservation");
    assert_eq!(
        p.catalog
            .refs
            .lock()
            .unwrap()
            .get(&id_of(&copied["id"]))
            .copied(),
        Some((id_of(&copied["reservation_id"]), ReferenceState::Confirmed))
    );
    assert_eq!(
        *p.catalog.reserve_kinds.lock().unwrap(),
        vec![ReferenceKind::PlanItem, ReferenceKind::PlanItem]
    );
    assert!(
        due(&p.a).await.is_empty(),
        "the copy finished its attach op"
    );
    assert_eq!(p.catalog.releases(), 0, "nothing was released");
}
