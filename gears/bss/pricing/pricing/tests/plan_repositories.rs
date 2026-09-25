//! Phase 3 repositories on `SQLite`: plans, revisions and items.
//!
//! Every conditional write is raced by two real writers on one database file, each on its own
//! connection, released together by a barrier: exactly one may win, and the loser must see the
//! write's own conflict code rather than a driver error or a second success.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use bss_approval::{Store, Unit, UnitState};
use bss_pricing::{
    domain::plan,
    infra::storage::{
        RepoError,
        entity::{plan as plan_e, plan_item, plan_revision, price_book, price_book_entry},
        repo::{
            approval_repo::PricingApprovalStore, book_repo, plan_item_repo, plan_repo,
            plan_revision_repo, price_book_entry_repo, price_repo,
        },
    },
};
use std::{future::Future, pin::Pin, sync::Arc};
use toolkit_db::secure::{AccessScope, TxConfig};
use toolkit_db::{ConnectOpts, DBProvider, Db, DbError, DbTx};
use uuid::Uuid;
mod storage_support;
use storage_support::{at, test_db};

fn book(tenant: Uuid) -> price_book::Model {
    price_book::Model {
        id: Uuid::new_v4(),
        tenant_id: tenant,
        code: format!("book-{}", Uuid::new_v4()),
        name: "Default EUR".into(),
        currency: "EUR".into(),
        valid_from: None,
        valid_until: None,
        version: 1,
        created_at: at(9),
        updated_at: at(9),
    }
}
fn entry(b: &price_book::Model, sku: Uuid) -> price_book_entry::Model {
    price_book_entry::Model {
        id: Uuid::new_v4(),
        tenant_id: b.tenant_id,
        book_id: b.id,
        sku_id: sku,
        charge_kind: "usage".into(),
        period: None,
        dimension_key: None,
        invoice_line_override: None,
        reservation_id: Uuid::new_v4(),
        reference_state: "confirmed".into(),
        version: 1,
        created_at: at(9),
        updated_at: at(9),
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
        created_at: at(9),
        updated_at: at(9),
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
        available_from: None,
        pending_unit_id: None,
        approved_by_unit_id: None,
        published_at: None,
        version: 1,
        created_by: Uuid::new_v4(),
        created_at: at(9),
        updated_at: at(9),
    }
}
fn item(r: &plan_revision::Model, e: &price_book_entry::Model) -> plan_item::Model {
    plan_item::Model {
        id: Uuid::new_v4(),
        tenant_id: r.tenant_id,
        revision_id: r.id,
        sku_id: e.sku_id,
        price_book_entry_id: Some(e.id),
        treatment: "paid".into(),
        included_qty: None,
        qty_min: Some(1),
        reservation_id: None,
        reference_state: "unreserved".into(),
        version: 1,
        created_by: Uuid::new_v4(),
        created_at: at(9),
        updated_at: at(9),
    }
}
fn date(s: &str) -> time::Date {
    time::Date::parse(s, &time::format_description::well_known::Iso8601::DATE).unwrap()
}
/// A pending approval unit of `kind`, so a lock or a request can name it.
async fn unit(db: &DBProvider<DbError>, scope: &AccessScope, tenant: Uuid, kind: &str) -> Uuid {
    let id = Uuid::new_v4();
    let scope = scope.clone();
    let kind = kind.to_owned();
    price_repo::transaction(&db.db(), move |tx| {
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
                    submitted_at: at(9),
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

// ---------------------------------------------------------------- the two-writer race

type Work = Arc<
    dyn for<'a> Fn(&'a DbTx<'a>) -> Pin<Box<dyn Future<Output = Result<(), RepoError>> + Send + 'a>>
        + Send
        + Sync,
>;
fn db_error(e: &RepoError) -> Option<&sea_orm::DbErr> {
    match e {
        RepoError::Driver { source, .. } => Some(source),
        _ => None,
    }
}
async fn writer(db: Db, work: Work, barrier: Arc<tokio::sync::Barrier>) -> Result<(), RepoError> {
    let mut attempt = 0;
    db.transaction_with_retry(TxConfig::serializable(), db_error, move |tx| {
        attempt += 1;
        let first = attempt == 1;
        let barrier = Arc::clone(&barrier);
        let work = Arc::clone(&work);
        Box::pin(async move {
            if first {
                barrier.wait().await;
            }
            work(tx).await
        })
    })
    .await
}
/// Run `work` on two connections to one file at once; exactly one wins and the other sees `code`.
async fn exactly_one_wins(db: &DBProvider<DbError>, dsn: &str, code: &str, work: Work) {
    let other = toolkit_db::connect_db(
        dsn,
        ConnectOpts {
            max_conns: Some(1),
            min_conns: Some(1),
            ..ConnectOpts::default()
        },
    )
    .await
    .unwrap();
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let (a, b) = tokio::join!(
        writer(db.db(), Arc::clone(&work), Arc::clone(&barrier)),
        writer(other, work, barrier)
    );
    assert_eq!(
        usize::from(a.is_ok()) + usize::from(b.is_ok()),
        1,
        "{code}: {a:?} / {b:?}"
    );
    let loser = if let Err(e) = a { e } else { b.unwrap_err() };
    assert!(
        matches!(&loser, RepoError::Conflict { code: c } if *c == code),
        "{code}: the loser saw {loser:?}"
    );
}
fn lost(won: bool) -> Result<(), RepoError> {
    if won {
        Ok(())
    } else {
        Err(RepoError::Conflict { code: "LOCK_LOST" })
    }
}
fn conflict<T: std::fmt::Debug>(result: Result<T, RepoError>, code: &str) {
    match result {
        Err(RepoError::Conflict { code: c }) if c == code => {}
        other => panic!("expected {code}, got {other:?}"),
    }
}

/// A tenant with a book, an entry, a plan and a draft rev 1, all on one file.
struct World {
    db: DBProvider<DbError>,
    scope: AccessScope,
    tenant: Uuid,
    dsn: String,
    book: price_book::Model,
    entry: price_book_entry::Model,
    plan: plan_e::Model,
    revision: plan_revision::Model,
}
async fn world() -> World {
    let (db, scope, tenant, dsn) = test_db().await;
    let conn = db.conn().unwrap();
    let b = book_repo::insert(&conn, &scope, book(tenant))
        .await
        .unwrap();
    let e = price_book_entry_repo::insert(&conn, &scope, entry(&b, Uuid::new_v4()))
        .await
        .unwrap();
    let p = plan_repo::insert(&conn, &scope, plan(tenant, "pro"))
        .await
        .unwrap();
    let r = plan_revision_repo::insert(&conn, &scope, revision(&p, &b, 1))
        .await
        .unwrap();
    World {
        db,
        scope,
        tenant,
        dsn,
        book: b,
        entry: e,
        plan: p,
        revision: r,
    }
}

// ---------------------------------------------------------------- plans

#[tokio::test]
async fn plan_round_trips_and_its_code_is_unique_per_tenant() {
    let w = world().await;
    let conn = w.db.conn().unwrap();
    assert_eq!(
        plan_repo::find(&conn, &w.scope, w.tenant, w.plan.id)
            .await
            .unwrap(),
        Some(w.plan.clone())
    );
    conflict(
        plan_repo::insert(&conn, &w.scope, plan(w.tenant, "pro")).await,
        "PLAN_CODE_TAKEN",
    );
    let other = plan_repo::insert(&conn, &w.scope, plan(w.tenant, "basic"))
        .await
        .unwrap();
    let listed: Vec<_> = plan_repo::list(&conn, &w.scope, w.tenant)
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.code)
        .collect();
    assert_eq!(listed, vec!["basic".to_owned(), "pro".to_owned()]);
    // Another tenant may reuse the code and cannot read this tenant's plan.
    let foreign = Uuid::new_v4();
    let foreign_scope = AccessScope::for_tenant(foreign);
    plan_repo::insert(&conn, &foreign_scope, plan(foreign, "pro"))
        .await
        .unwrap();
    assert!(
        plan_repo::find(&conn, &foreign_scope, w.tenant, other.id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn plan_rename_is_a_conditional_write() {
    let w = world().await;
    let (tenant, id, scope) = (w.tenant, w.plan.id, w.scope.clone());
    exactly_one_wins(
        &w.db,
        &w.dsn,
        "STALE_REVISION",
        Arc::new(move |tx| {
            let scope = scope.clone();
            Box::pin(async move {
                plan_repo::rename(tx, &scope, tenant, id, 1, "Pro 2".into(), at(10)).await
            })
        }),
    )
    .await;
    let got = plan_repo::find(&w.db.conn().unwrap(), &w.scope, tenant, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((got.name.as_str(), got.version), ("Pro 2", 2));
}

#[tokio::test]
async fn plan_publish_projection_is_a_conditional_write() {
    let w = world().await;
    let (tenant, id, scope) = (w.tenant, w.plan.id, w.scope.clone());
    exactly_one_wins(
        &w.db,
        &w.dsn,
        "STALE_REVISION",
        Arc::new(move |tx| {
            let scope = scope.clone();
            Box::pin(
                async move { plan_repo::set_published(tx, &scope, tenant, id, 1, 3, at(10)).await },
            )
        }),
    )
    .await;
    assert_eq!(
        plan_repo::find(&w.db.conn().unwrap(), &w.scope, tenant, id)
            .await
            .unwrap()
            .unwrap()
            .published_rev,
        Some(3)
    );
}

// ---------------------------------------------------------------- revisions

#[tokio::test]
async fn revision_parents_are_tenant_scoped() {
    let w = world().await;
    let conn = w.db.conn().unwrap();
    let foreign = Uuid::new_v4();
    let foreign_scope = AccessScope::for_tenant(foreign);
    let mut r = revision(&w.plan, &w.book, 2);
    r.tenant_id = foreign;
    conflict(
        plan_revision_repo::insert(&conn, &foreign_scope, r).await,
        "PLAN_NOT_FOUND",
    );
    let theirs = plan_repo::insert(&conn, &foreign_scope, plan(foreign, "pro"))
        .await
        .unwrap();
    // Their plan, our book.
    conflict(
        plan_revision_repo::insert(&conn, &foreign_scope, revision(&theirs, &w.book, 1)).await,
        "BOOK_NOT_FOUND",
    );
    assert!(
        plan_revision_repo::find(&conn, &foreign_scope, w.tenant, w.revision.id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn one_open_and_one_published_revision_per_plan_and_rev_numbers_are_unique() {
    let w = world().await;
    let conn = w.db.conn().unwrap();
    // A superseded rev 1 joins no partial index, so only the rev_no key can refuse it.
    let mut same_no = revision(&w.plan, &w.book, 1);
    same_no.state = "superseded".into();
    conflict(
        plan_revision_repo::insert(&conn, &w.scope, same_no).await,
        "REVISION_NO_TAKEN",
    );
    // rev 1 is the open draft: neither a second draft nor a pending revision may join it.
    conflict(
        plan_revision_repo::insert(&conn, &w.scope, revision(&w.plan, &w.book, 2)).await,
        "REVISION_DRAFT_EXISTS",
    );
    let u = unit(&w.db, &w.scope, w.tenant, "plan_revision").await;
    let mut pending = revision(&w.plan, &w.book, 2);
    pending.state = "pending".into();
    pending.pending_unit_id = Some(u);
    conflict(
        plan_revision_repo::insert(&conn, &w.scope, pending).await,
        "REVISION_DRAFT_EXISTS",
    );
    // Published and superseded revisions sit beside the open one; only one is published.
    let mut published = revision(&w.plan, &w.book, 2);
    published.state = "published".into();
    plan_revision_repo::insert(&conn, &w.scope, published)
        .await
        .unwrap();
    let mut second = revision(&w.plan, &w.book, 3);
    second.state = "published".into();
    conflict(
        plan_revision_repo::insert(&conn, &w.scope, second).await,
        "REVISION_PUBLISHED_EXISTS",
    );
    for rev_no in [4, 5] {
        let mut old = revision(&w.plan, &w.book, rev_no);
        old.state = "superseded".into();
        plan_revision_repo::insert(&conn, &w.scope, old)
            .await
            .unwrap();
    }
    let numbers: Vec<_> = plan_revision_repo::for_plan(&conn, &w.scope, w.tenant, w.plan.id)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.rev_no)
        .collect();
    assert_eq!(numbers, vec![1, 2, 4, 5]);
}

#[tokio::test]
async fn revision_state_is_the_enums_vocabulary_and_round_trips() {
    let w = world().await;
    let conn = w.db.conn().unwrap();
    let other = plan_repo::insert(&conn, &w.scope, plan(w.tenant, "other"))
        .await
        .unwrap();
    let mut odd = revision(&other, &w.book, 1);
    odd.state = "retired".into();
    assert!(
        plan_revision_repo::insert(&conn, &w.scope, odd)
            .await
            .is_err()
    );
    let mut dated = revision(&other, &w.book, 1);
    dated.available_from = Some(date("2026-10-01"));
    assert_eq!(
        plan_revision_repo::insert(&conn, &w.scope, dated.clone())
            .await
            .unwrap(),
        dated
    );
}

#[tokio::test]
async fn revision_draft_edit_is_a_conditional_write_on_an_unlocked_draft() {
    let w = world().await;
    let mut edited = w.revision.clone();
    edited.available_from = Some(date("2026-10-01"));
    let scope = w.scope.clone();
    exactly_one_wins(
        &w.db,
        &w.dsn,
        "STALE_REVISION",
        Arc::new(move |tx| {
            let scope = scope.clone();
            let edited = edited.clone();
            Box::pin(async move { plan_revision_repo::update_draft(tx, &scope, edited).await })
        }),
    )
    .await;
    let conn = w.db.conn().unwrap();
    let got = plan_revision_repo::find(&conn, &w.scope, w.tenant, w.revision.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (got.available_from, got.version),
        (Some(date("2026-10-01")), 2)
    );
    // A book of another tenant is refused before the write.
    let foreign = Uuid::new_v4();
    let theirs = book_repo::insert(&conn, &AccessScope::for_tenant(foreign), book(foreign))
        .await
        .unwrap();
    let mut moved = got.clone();
    moved.book_id = theirs.id;
    conflict(
        plan_revision_repo::update_draft(&conn, &w.scope, moved).await,
        "BOOK_NOT_FOUND",
    );
    // A locked revision is no longer editable.
    let u = unit(&w.db, &w.scope, w.tenant, "plan_revision").await;
    assert!(
        plan_revision_repo::try_lock(&conn, &w.scope, w.tenant, w.revision.id, u, 2)
            .await
            .unwrap()
    );
    let mut late = got;
    late.version = 3;
    conflict(
        plan_revision_repo::update_draft(&conn, &w.scope, late).await,
        "STALE_REVISION",
    );
}

#[tokio::test]
async fn revision_lock_is_conditional_and_names_an_existing_unit() {
    let w = world().await;
    let conn = w.db.conn().unwrap();
    conflict(
        plan_revision_repo::try_lock(&conn, &w.scope, w.tenant, w.revision.id, Uuid::new_v4(), 1)
            .await,
        "UNIT_NOT_FOUND",
    );
    let u = unit(&w.db, &w.scope, w.tenant, "plan_revision").await;
    let (tenant, id, scope) = (w.tenant, w.revision.id, w.scope.clone());
    exactly_one_wins(
        &w.db,
        &w.dsn,
        "LOCK_LOST",
        Arc::new(move |tx| {
            let scope = scope.clone();
            Box::pin(async move {
                lost(plan_revision_repo::try_lock(tx, &scope, tenant, id, u, 1).await?)
            })
        }),
    )
    .await;
    let got = plan_revision_repo::find(&conn, &w.scope, w.tenant, w.revision.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (got.state.as_str(), got.pending_unit_id, got.version),
        ("pending", Some(u), 2)
    );
}

#[tokio::test]
async fn revision_unlock_returns_the_owning_units_revision_to_draft() {
    let w = world().await;
    let conn = w.db.conn().unwrap();
    let u = unit(&w.db, &w.scope, w.tenant, "plan_revision").await;
    assert!(
        plan_revision_repo::try_lock(&conn, &w.scope, w.tenant, w.revision.id, u, 1)
            .await
            .unwrap()
    );
    conflict(
        plan_revision_repo::unlock(&conn, &w.scope, w.tenant, w.revision.id, Uuid::new_v4()).await,
        "STALE_REVISION",
    );
    let (tenant, id, scope) = (w.tenant, w.revision.id, w.scope.clone());
    exactly_one_wins(
        &w.db,
        &w.dsn,
        "STALE_REVISION",
        Arc::new(move |tx| {
            let scope = scope.clone();
            Box::pin(async move { plan_revision_repo::unlock(tx, &scope, tenant, id, u).await })
        }),
    )
    .await;
    let got = plan_revision_repo::find(&conn, &w.scope, w.tenant, w.revision.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (got.state.as_str(), got.pending_unit_id, got.version),
        ("draft", None, 3)
    );
}

#[tokio::test]
async fn revision_publish_is_conditional_on_the_owning_unit_and_one_published_per_plan() {
    let w = world().await;
    let conn = w.db.conn().unwrap();
    let u = unit(&w.db, &w.scope, w.tenant, "plan_revision").await;
    assert!(
        plan_revision_repo::try_lock(&conn, &w.scope, w.tenant, w.revision.id, u, 1)
            .await
            .unwrap()
    );
    conflict(
        plan_revision_repo::publish(
            &conn,
            &w.scope,
            w.tenant,
            w.revision.id,
            Uuid::new_v4(),
            at(10),
        )
        .await,
        "REVISION_NOT_PENDING",
    );
    let (tenant, id, scope) = (w.tenant, w.revision.id, w.scope.clone());
    exactly_one_wins(
        &w.db,
        &w.dsn,
        "REVISION_NOT_PENDING",
        Arc::new(move |tx| {
            let scope = scope.clone();
            Box::pin(
                async move { plan_revision_repo::publish(tx, &scope, tenant, id, u, at(10)).await },
            )
        }),
    )
    .await;
    let got = plan_revision_repo::find(&conn, &w.scope, w.tenant, w.revision.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            got.state.as_str(),
            got.pending_unit_id,
            got.approved_by_unit_id,
            got.published_at
        ),
        ("published", None, Some(u), Some(at(10)))
    );
    // A second revision cannot publish beside it until it is superseded.
    let next = plan_revision_repo::insert(&conn, &w.scope, revision(&w.plan, &w.book, 2))
        .await
        .unwrap();
    let u2 = unit(&w.db, &w.scope, w.tenant, "plan_revision").await;
    assert!(
        plan_revision_repo::try_lock(&conn, &w.scope, w.tenant, next.id, u2, 1)
            .await
            .unwrap()
    );
    conflict(
        plan_revision_repo::publish(&conn, &w.scope, w.tenant, next.id, u2, at(11)).await,
        "REVISION_PUBLISHED_EXISTS",
    );
    plan_revision_repo::supersede(
        &conn,
        &w.scope,
        w.tenant,
        w.revision.id,
        got.version,
        at(11),
    )
    .await
    .unwrap();
    plan_revision_repo::publish(&conn, &w.scope, w.tenant, next.id, u2, at(11))
        .await
        .unwrap();
}

#[tokio::test]
async fn revision_supersede_is_a_conditional_write_on_a_published_revision() {
    let w = world().await;
    let conn = w.db.conn().unwrap();
    // A draft is not superseded.
    conflict(
        plan_revision_repo::supersede(&conn, &w.scope, w.tenant, w.revision.id, 1, at(10)).await,
        "STALE_REVISION",
    );
    let mut published = revision(&w.plan, &w.book, 2);
    published.state = "published".into();
    let published = plan_revision_repo::insert(&conn, &w.scope, published)
        .await
        .unwrap();
    let (tenant, id, scope) = (w.tenant, published.id, w.scope.clone());
    exactly_one_wins(
        &w.db,
        &w.dsn,
        "STALE_REVISION",
        Arc::new(move |tx| {
            let scope = scope.clone();
            Box::pin(async move {
                plan_revision_repo::supersede(tx, &scope, tenant, id, 1, at(10)).await
            })
        }),
    )
    .await;
    assert_eq!(
        plan_revision_repo::find(&conn, &w.scope, w.tenant, published.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        "superseded"
    );
}

#[tokio::test]
async fn revision_delete_is_a_conditional_write_on_an_empty_unlocked_draft() {
    let w = world().await;
    let conn = w.db.conn().unwrap();
    let i = plan_item_repo::insert(&conn, &w.scope, item(&w.revision, &w.entry))
        .await
        .unwrap();
    // Items go first: the revision refuses to leave them behind.
    conflict(
        plan_revision_repo::delete_draft(&conn, &w.scope, w.tenant, w.revision.id, 1).await,
        "STALE_REVISION",
    );
    plan_item_repo::delete_draft(&conn, &w.scope, w.tenant, i.id, 1)
        .await
        .unwrap();
    let (tenant, id, scope) = (w.tenant, w.revision.id, w.scope.clone());
    exactly_one_wins(
        &w.db,
        &w.dsn,
        "STALE_REVISION",
        Arc::new(move |tx| {
            let scope = scope.clone();
            Box::pin(
                async move { plan_revision_repo::delete_draft(tx, &scope, tenant, id, 1).await },
            )
        }),
    )
    .await;
    assert!(
        plan_revision_repo::find(&conn, &w.scope, w.tenant, w.revision.id)
            .await
            .unwrap()
            .is_none()
    );
}

// ---------------------------------------------------------------- items

#[tokio::test]
async fn item_parents_are_tenant_scoped_and_the_revision_must_be_an_unlocked_draft() {
    let w = world().await;
    let conn = w.db.conn().unwrap();
    let foreign = Uuid::new_v4();
    let foreign_scope = AccessScope::for_tenant(foreign);
    let mut theirs = item(&w.revision, &w.entry);
    theirs.tenant_id = foreign;
    conflict(
        plan_item_repo::insert(&conn, &foreign_scope, theirs).await,
        "REVISION_NOT_FOUND",
    );
    let mut ghost = item(&w.revision, &w.entry);
    ghost.price_book_entry_id = Some(Uuid::new_v4());
    conflict(
        plan_item_repo::insert(&conn, &w.scope, ghost).await,
        "ENTRY_NOT_FOUND",
    );
    let u = unit(&w.db, &w.scope, w.tenant, "plan_revision").await;
    assert!(
        plan_revision_repo::try_lock(&conn, &w.scope, w.tenant, w.revision.id, u, 1)
            .await
            .unwrap()
    );
    conflict(
        plan_item_repo::insert(&conn, &w.scope, item(&w.revision, &w.entry)).await,
        "REVISION_NOT_DRAFT",
    );
}

#[tokio::test]
async fn item_round_trips_one_per_sku_and_its_columns_are_checked() {
    let w = world().await;
    let conn = w.db.conn().unwrap();
    let mut included = item(&w.revision, &w.entry);
    included.treatment = "included".into();
    included.included_qty = Some("100.50".into());
    included.qty_min = None;
    let got = plan_item_repo::insert(&conn, &w.scope, included.clone())
        .await
        .unwrap();
    assert_eq!(got, included);
    assert_eq!(got.included_qty.as_deref(), Some("100.50"), "exact text");
    conflict(
        plan_item_repo::insert(
            &conn,
            &w.scope,
            plan_item::Model {
                id: Uuid::new_v4(),
                ..included.clone()
            },
        )
        .await,
        "ITEM_SKU_TAKEN",
    );
    // An included item may name no entry; a paid or optional one may not.
    let mut free = item(&w.revision, &w.entry);
    free.sku_id = Uuid::new_v4();
    free.price_book_entry_id = None;
    free.treatment = "included".into();
    plan_item_repo::insert(&conn, &w.scope, free.clone())
        .await
        .unwrap();
    for treatment in ["paid", "optional"] {
        let mut priced = free.clone();
        priced.id = Uuid::new_v4();
        priced.sku_id = Uuid::new_v4();
        priced.treatment = treatment.into();
        assert!(
            plan_item_repo::insert(&conn, &w.scope, priced)
                .await
                .is_err(),
            "{treatment} without an entry"
        );
    }
    let bad = |f: &dyn Fn(&mut plan_item::Model)| {
        let mut m = free.clone();
        m.id = Uuid::new_v4();
        m.sku_id = Uuid::new_v4();
        f(&mut m);
        m
    };
    for (what, m) in [
        ("treatment", bad(&|m| m.treatment = "free".into())),
        (
            "reference state",
            bad(&|m| m.reference_state = "held".into()),
        ),
        ("negative qty_min", bad(&|m| m.qty_min = Some(-1))),
        (
            "signed quantity",
            bad(&|m| m.included_qty = Some("-1".into())),
        ),
        ("exponent", bad(&|m| m.included_qty = Some("1e5".into()))),
        ("trailing dot", bad(&|m| m.included_qty = Some("1.".into()))),
        ("empty", bad(&|m| m.included_qty = Some(String::new()))),
    ] {
        assert!(
            plan_item_repo::insert(&conn, &w.scope, m).await.is_err(),
            "the columns must refuse a bad {what}"
        );
    }
    assert_eq!(
        plan_item_repo::for_revision(&conn, &w.scope, w.tenant, w.revision.id)
            .await
            .unwrap()
            .len(),
        2
    );
    assert!(
        plan_item_repo::names_entry(&conn, &w.scope, w.tenant, w.entry.id)
            .await
            .unwrap()
    );
    assert!(
        !plan_item_repo::names_entry(&conn, &w.scope, w.tenant, Uuid::new_v4())
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn item_draft_edit_is_a_conditional_write_while_its_revision_is_a_draft() {
    let w = world().await;
    let conn = w.db.conn().unwrap();
    let i = plan_item_repo::insert(&conn, &w.scope, item(&w.revision, &w.entry))
        .await
        .unwrap();
    let mut edited = i.clone();
    edited.treatment = "optional".into();
    edited.qty_min = Some(0);
    let scope = w.scope.clone();
    let race = edited.clone();
    exactly_one_wins(
        &w.db,
        &w.dsn,
        "STALE_REVISION",
        Arc::new(move |tx| {
            let scope = scope.clone();
            let race = race.clone();
            Box::pin(async move { plan_item_repo::update_draft(tx, &scope, race).await })
        }),
    )
    .await;
    let got = plan_item_repo::find(&conn, &w.scope, w.tenant, i.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (got.treatment.as_str(), got.qty_min, got.version),
        ("optional", Some(0), 2)
    );
    let mut ghost = got.clone();
    ghost.price_book_entry_id = Some(Uuid::new_v4());
    conflict(
        plan_item_repo::update_draft(&conn, &w.scope, ghost).await,
        "ENTRY_NOT_FOUND",
    );
    // Once the revision is locked, its items are fixed.
    let u = unit(&w.db, &w.scope, w.tenant, "plan_revision").await;
    assert!(
        plan_revision_repo::try_lock(&conn, &w.scope, w.tenant, w.revision.id, u, 1)
            .await
            .unwrap()
    );
    conflict(
        plan_item_repo::update_draft(&conn, &w.scope, got.clone()).await,
        "STALE_REVISION",
    );
    conflict(
        plan_item_repo::delete_draft(&conn, &w.scope, w.tenant, i.id, got.version).await,
        "STALE_REVISION",
    );
}

#[tokio::test]
async fn item_reference_moves_in_any_revision_state_at_the_observed_version() {
    let w = world().await;
    let conn = w.db.conn().unwrap();
    let i = plan_item_repo::insert(&conn, &w.scope, item(&w.revision, &w.entry))
        .await
        .unwrap();
    // A published revision's item still takes its reference receipt (D-413's attach).
    let u = unit(&w.db, &w.scope, w.tenant, "plan_revision").await;
    assert!(
        plan_revision_repo::try_lock(&conn, &w.scope, w.tenant, w.revision.id, u, 1)
            .await
            .unwrap()
    );
    plan_revision_repo::publish(&conn, &w.scope, w.tenant, w.revision.id, u, at(10))
        .await
        .unwrap();
    let receipt = Uuid::new_v4();
    let (tenant, id, scope) = (w.tenant, i.id, w.scope.clone());
    exactly_one_wins(
        &w.db,
        &w.dsn,
        "STALE_REVISION",
        Arc::new(move |tx| {
            let scope = scope.clone();
            Box::pin(async move {
                plan_item_repo::set_reference(
                    tx,
                    &scope,
                    tenant,
                    id,
                    1,
                    plan::ReferenceState::Confirmed,
                    Some(receipt),
                    at(11),
                )
                .await
            })
        }),
    )
    .await;
    let got = plan_item_repo::find(&conn, &w.scope, w.tenant, i.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            got.reference_state.as_str(),
            got.reservation_id,
            got.version
        ),
        ("confirmed", Some(receipt), 2)
    );
}

#[tokio::test]
async fn item_delete_is_a_conditional_write() {
    let w = world().await;
    let conn = w.db.conn().unwrap();
    let i = plan_item_repo::insert(&conn, &w.scope, item(&w.revision, &w.entry))
        .await
        .unwrap();
    let (tenant, id, scope) = (w.tenant, i.id, w.scope.clone());
    exactly_one_wins(
        &w.db,
        &w.dsn,
        "STALE_REVISION",
        Arc::new(move |tx| {
            let scope = scope.clone();
            Box::pin(async move { plan_item_repo::delete_draft(tx, &scope, tenant, id, 1).await })
        }),
    )
    .await;
    assert!(
        plan_item_repo::find(&conn, &w.scope, w.tenant, i.id)
            .await
            .unwrap()
            .is_none()
    );
}

// ---------------------------------------------------------------- conflict vocabulary

#[test]
fn phase_3_unique_messages_match_both_engines() {
    use bss_pricing::infra::storage::repo::unique_code;
    for (message, code) in [
        (
            "duplicate key value violates unique constraint \"pricing_plan_code\"",
            "PLAN_CODE_TAKEN",
        ),
        (
            "UNIQUE constraint failed: pricing_plan.tenant_id, pricing_plan.code",
            "PLAN_CODE_TAKEN",
        ),
        (
            "duplicate key value violates unique constraint \"pricing_plan_revision_no\"",
            "REVISION_NO_TAKEN",
        ),
        (
            "UNIQUE constraint failed: pricing_plan_revision.plan_id, pricing_plan_revision.rev_no",
            "REVISION_NO_TAKEN",
        ),
        (
            "duplicate key value violates unique constraint \"pricing_plan_revision_open\"",
            "REVISION_DRAFT_EXISTS",
        ),
        (
            "duplicate key value violates unique constraint \"pricing_plan_revision_published\"",
            "REVISION_PUBLISHED_EXISTS",
        ),
        (
            "duplicate key value violates unique constraint \"pricing_plan_item_sku\"",
            "ITEM_SKU_TAKEN",
        ),
        (
            "UNIQUE constraint failed: pricing_plan_item.revision_id, pricing_plan_item.sku_id",
            "ITEM_SKU_TAKEN",
        ),
    ] {
        assert_eq!(unique_code(message), Some(code), "{message}");
    }
    // SQLite names only the columns of a partial index: the two single-column revision indexes
    // read alike, so the shared matcher leaves them to the revision repository, which knows the
    // state it wrote.
    assert_eq!(
        unique_code("UNIQUE constraint failed: pricing_plan_revision.plan_id"),
        None
    );
}
