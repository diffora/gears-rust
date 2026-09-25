//! Products' in-process registry reads its own database through `Db::conn()`, which toolkit-db
//! refuses on a task that is inside a transaction (`ConnRequestedInsideTx`). The approval
//! subjects read SKUs inside the unit's transaction (D-408; D-402's dated metering reads), so the
//! doors must not hand the registry their transaction's task. The double here refuses exactly as
//! Products does, so a submit, a publish or a vote that reads a SKU on the transaction's task
//! answers 503 instead of recording its unit.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod plan_support;
use bss_pricing::infra::storage::repo::{price_book_entry_repo, price_repo};
use bss_products_sdk::{
    ReferenceRegistryV1,
    models::{ReferenceKind, ReferenceState, ReservationReceipt, Sku, SkuType, SkuVersion},
};
use plan_support::{Catalog, Fixture, book, entry, id_of, plan, scope};
use serde_json::{Value, json};
use std::sync::Arc;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// The catalog double behind Products' own first step: a connection to its own database, refused
/// inside the caller's transaction as `LocalReferenceRegistry` is.
struct InProcess {
    catalog: Arc<Catalog>,
    db: toolkit_db::DBProvider<toolkit_db::DbError>,
}
impl InProcess {
    fn connect(&self) -> Result<(), CanonicalError> {
        self.db
            .conn()
            .map(|_| ())
            .map_err(|e| CanonicalError::internal(e.to_string()).create())
    }
}
#[async_trait::async_trait]
impl ReferenceRegistryV1 for InProcess {
    async fn reserve(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        sku: Uuid,
        kind: ReferenceKind,
        ref_id: Uuid,
    ) -> Result<ReservationReceipt, CanonicalError> {
        self.connect()?;
        self.catalog.reserve(ctx, tenant, sku, kind, ref_id).await
    }
    async fn confirm(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        id: Uuid,
    ) -> Result<(), CanonicalError> {
        self.connect()?;
        self.catalog.confirm(ctx, tenant, id).await
    }
    async fn release(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        id: Uuid,
    ) -> Result<(), CanonicalError> {
        self.connect()?;
        self.catalog.release(ctx, tenant, id).await
    }
    async fn states(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        ids: &[Uuid],
    ) -> Result<Vec<(Uuid, ReferenceState)>, CanonicalError> {
        self.connect()?;
        self.catalog.states(ctx, tenant, ids).await
    }
    async fn sku_for_write(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        id: Uuid,
    ) -> Result<Sku, CanonicalError> {
        self.connect()?;
        self.catalog.sku_for_write(ctx, tenant, id).await
    }
    async fn sku_version_as_of(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        id: Uuid,
        date: time::Date,
    ) -> Result<Option<SkuVersion>, CanonicalError> {
        self.connect()?;
        self.catalog.sku_version_as_of(ctx, tenant, id, date).await
    }
}

async fn setup() -> (Fixture, Arc<Catalog>) {
    let catalog = Arc::new(Catalog::default());
    let (db, _, _, _) = plan_support::entry_support::test_db().await;
    let f = Fixture::new(Arc::new(InProcess {
        catalog: catalog.clone(),
        db,
    }))
    .await;
    (f, catalog)
}
async fn quorum(f: &Fixture, kind: &str, quorum: u32) {
    let (_, _, tag) = f
        .call("GET", "/approval-policy", json!({}), None, None)
        .await;
    let (s, b, _) = f
        .call(
            "PUT",
            "/approval-policy",
            json!({"kind":kind,"quorum":quorum}),
            Some(&tag),
            None,
        )
        .await;
    assert_eq!(s, 200, "{b}");
}
async fn draft(f: &Fixture, entry: &str, from: &str, key: &str) -> Value {
    let (s, b, _) = f
        .call(
            "POST",
            &format!("/price-book-entries/{entry}/prices"),
            json!({"model":"per_unit","price":{"rate":"0.10"},"eligibility":"all","effective_from":from}),
            None,
            Some(key),
        )
        .await;
    assert_eq!(s, 201, "{b}");
    b["items"][0].clone()
}

/// A `prices` unit reads each entry SKU (its descriptors, D-408) and a usage chain's dated
/// metering (D-402) inside its transaction: publish-changes at quorum 0, and a submit then an
/// approving vote at quorum 1, record and apply their units.
#[tokio::test]
async fn a_prices_unit_is_recorded_and_applied_through_an_in_process_registry() {
    let (f, catalog) = setup().await;
    let book = book(&f, "eur").await;
    let sku = catalog.sku(SkuType::Usage);
    let (s, e, _) = f
        .call(
            "POST",
            &format!("/price-books/{book}/entries"),
            json!({"sku_id":sku}),
            None,
            Some("entry"),
        )
        .await;
    assert_eq!(s, 201, "{e}");
    let e = e["id"].as_str().unwrap().to_owned();
    quorum(&f, "prices", 0).await;
    draft(&f, &e, "2031-03-01", "first").await;
    let (s, b, _) = f
        .call(
            "POST",
            &format!("/price-books/{book}/publish-changes"),
            json!({}),
            None,
            Some("publish"),
        )
        .await;
    assert_eq!(s, 201, "{b}");
    assert_eq!(b["applied"], true, "{b}");
    quorum(&f, "prices", 1).await;
    let price = draft(&f, &e, "2031-06-01", "second").await;
    let (s, receipt, _) = f
        .call(
            "POST",
            &format!("/prices/{}/submit", price["id"].as_str().unwrap()),
            json!({}),
            None,
            Some("submit"),
        )
        .await;
    assert_eq!(s, 201, "{receipt}");
    let (s, b, _) = f
        .call_as(
            &f.user(),
            "POST",
            &format!(
                "/approval-units/{}/approve",
                receipt["unit"]["id"].as_str().unwrap()
            ),
            json!({"generation":1}),
            None,
            Some("approve"),
        )
        .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["outcome"], "applied", "{b}");
}

/// A `plan_revision` unit judges the revision with fresh SKU reads at submit and again at apply,
/// inside its transaction: the submit records the unit and the approving vote publishes it.
#[tokio::test]
async fn a_plan_revision_is_submitted_and_applied_through_an_in_process_registry() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let (created, revision) = plan(&f, "pro", eur).await;
    let sku = catalog.sku(SkuType::Usage);
    let priced = entry(&f, eur, sku, "usage", None).await;
    let conn = f.db.conn().unwrap();
    let stored = price_book_entry_repo::find(&conn, &scope(&f), f.ctx.subject_tenant_id(), priced)
        .await
        .unwrap()
        .unwrap();
    let mut approved = plan_support::entry_support::price(&stored);
    approved.state = "approved".into();
    approved.effective_from = time::Date::parse(
        "2020-01-01",
        &time::format_description::well_known::Iso8601::DATE,
    )
    .unwrap();
    price_repo::insert(&conn, &scope(&f), approved)
        .await
        .unwrap();
    let (s, b, _) = f
        .call(
            "POST",
            &format!("/plan-revisions/{revision}/items"),
            json!({"sku_id":sku,"price_book_entry_id":priced,"treatment":"paid"}),
            None,
            Some("item"),
        )
        .await;
    assert_eq!(s, 201, "{b}");
    quorum(&f, "plan_revision", 1).await;
    let (s, receipt, _) = f
        .call(
            "POST",
            &format!("/plan-revisions/{revision}/submit"),
            json!({}),
            None,
            Some("submit"),
        )
        .await;
    assert_eq!(s, 201, "{receipt}");
    assert_eq!(receipt["revision"]["state"], "pending", "{receipt}");
    let (s, b, _) = f
        .call_as(
            &f.user(),
            "POST",
            &format!(
                "/approval-units/{}/approve",
                receipt["unit"]["id"].as_str().unwrap()
            ),
            json!({"generation":1}),
            None,
            Some("approve"),
        )
        .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["outcome"], "applied", "{b}");
    let (s, plan_now, _) = f
        .call(
            "GET",
            &format!("/plans/{}", id_of(&created["id"])),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 200, "{plan_now}");
    assert_eq!(plan_now["published_rev"], 1);
}
