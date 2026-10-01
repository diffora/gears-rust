//! Real acceptance service fixture; successful receipts come only from the command.
#![allow(dead_code, clippy::expect_used, clippy::unwrap_used)]
use crate::{plan_support as p, seam_support as s};
use bss_pricing::{
    api::{
        pricing_acceptance::PricingAcceptanceProvider, pricing_read::PricingReadProvider,
        sellability::SellabilityProvider,
    },
    config::SellerHoldPolicy,
    infra::{clock::Clock, commercial_terms::CommercialTermsService},
};
use bss_pricing_sdk::{
    acceptance::{CommandMeta, NewSaleQuery},
    digest::selected_bindings_digest,
    read::{CatalogRef, PricingReadV1, ResolveQuery},
};
use std::sync::Arc;
use toolkit_security::SecurityContext;

pub struct FixedClock(pub parking_lot::Mutex<time::OffsetDateTime>);
impl FixedClock {
    pub fn advance(&self, by: time::Duration) {
        *self.0.lock() += by;
    }
}
impl Clock for FixedClock {
    fn now(&self) -> time::OffsetDateTime {
        *self.0.lock()
    }
}
pub struct AcceptanceFixture {
    pub denied_ctx: SecurityContext,
    pub fixture: p::Fixture,
    pub ctx: SecurityContext,
    pub query: NewSaleQuery,
    pub meta: CommandMeta,
    pub sellability: SellabilityProvider,
    pub acceptance: PricingAcceptanceProvider,
    pub read: PricingReadProvider,
    pub clock: Arc<FixedClock>,
    pub catalog: Arc<p::Catalog>,
}
impl AcceptanceFixture {
    pub async fn new() -> Self {
        let (fixture, catalog) = p::setup().await;
        let book = p::book(&fixture, "acceptance").await;
        let sku = catalog.sku(bss_products_sdk::models::SkuType::Usage);
        let entry = p::policy_entry(&fixture, book, sku, "usage", None).await;
        s::put(
            &fixture,
            entry,
            s::Row {
                price: serde_json::json!({"rate":"10"}),
                ..s::Row::default()
            },
        )
        .await;
        let (plan, revision_id) = p::plan(&fixture, "acceptance", book).await;
        p::item(&fixture, revision_id, sku, Some(entry), "paid").await;
        p::publish(&fixture, p::id_of(&plan["id"]), revision_id).await;
        let mut content = catalog.content(sku);
        content.gl_code = Some("usage".into());
        content.tax_category = Some("standard".into());
        content.invoice_line_template = Some("{sku}".into());
        content.billing_timing = Some(bss_products_sdk::models::BillingTiming::Arrears);
        catalog.version(sku, 3, "2026-09-01", content);
        let ctx = fixture.ctx.clone();
        let enforcer = Arc::new(p::entry_support::enforcer_for(ctx.subject_tenant_id()));
        let read = PricingReadProvider::new(fixture.state.clone(), enforcer.clone());
        let mut query = s::sale_query();
        let resolved = read
            .resolve(
                &ctx,
                ResolveQuery {
                    catalog: CatalogRef {
                        tenant_id: ctx.subject_tenant_id(),
                    },
                    revision_id,
                    date: query.start_at.date(),
                    item_id: None,
                    pins: vec![],
                },
            )
            .await
            .unwrap();
        query.tenant_axes.seller_tenant_id = ctx.subject_tenant_id();
        query.plan_id = resolved.plan_id;
        query.plan_revision_id = resolved.revision_id;
        query.selections = resolved.cells.iter().map(|c| c.selection.clone()).collect();
        query.resolved_bindings_digest =
            selected_bindings_digest(&resolved, &query.selections).unwrap();
        let clock = Arc::new(FixedClock(parking_lot::Mutex::new(query.start_at)));
        let service = Arc::new(CommercialTermsService::new(
            fixture.state.clone(),
            enforcer,
            clock.clone(),
            SellerHoldPolicy::default(),
        ));
        Self {
            denied_ctx: p::holding(&fixture, "denied"),
            fixture,
            ctx,
            query,
            meta: CommandMeta {
                idempotency_key: "accept-1".into(),
            },
            sellability: SellabilityProvider::new(service.clone()),
            acceptance: PricingAcceptanceProvider::new(service),
            read,
            clock,
            catalog,
        }
    }
}

impl AcceptanceFixture {
    pub fn service(&self, policy: SellerHoldPolicy) -> Arc<CommercialTermsService> {
        Arc::new(CommercialTermsService::new(
            self.fixture.state.clone(),
            Arc::new(p::entry_support::enforcer_for(self.ctx.subject_tenant_id())),
            self.clock.clone(),
            policy,
        ))
    }
    pub async fn counts(&self) -> (i64, i64, i64) {
        use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
        let db = Database::connect(&self.fixture.dsn).await.unwrap();
        let row=db.query_one_raw(Statement::from_string(DbBackend::Sqlite,"SELECT (SELECT count(*) FROM pricing_acceptance) AS a, (SELECT count(*) FROM pricing_commercial_command) AS c, (SELECT count(*) FROM pricing_audit WHERE subject_kind='acceptance') AS audit")).await.unwrap().unwrap();
        (
            row.try_get("", "a").unwrap(),
            row.try_get("", "c").unwrap(),
            row.try_get("", "audit").unwrap(),
        )
    }
    pub async fn execute(&self, sql: &str) {
        use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
        Database::connect(&self.fixture.dsn)
            .await
            .unwrap()
            .execute_raw(Statement::from_string(DbBackend::Sqlite, sql))
            .await
            .unwrap();
    }
    pub async fn resolved(&self) -> bss_pricing_sdk::read::ResolvedBindings {
        self.read
            .resolve(
                &self.ctx,
                ResolveQuery {
                    catalog: CatalogRef {
                        tenant_id: self.ctx.subject_tenant_id(),
                    },
                    revision_id: self.query.plan_revision_id,
                    date: self.query.start_at.date(),
                    item_id: None,
                    pins: vec![],
                },
            )
            .await
            .unwrap()
    }
    pub fn hook(
        &self,
        sql: Vec<String>,
        advance: Option<time::Duration>,
        repeat: bool,
    ) -> Arc<MeterHook> {
        let hook = Arc::new(MeterHook {
            dsn: self.fixture.dsn.to_string(),
            sql,
            once: std::sync::atomic::AtomicBool::new(false),
            repeat,
            clock: self.clock.clone(),
            advance,
            provider: p::entry_support::policy_support::MeterProvider::default(),
        });
        self.fixture
            .state
            .hub
            .register::<dyn bss_pricing_sdk::meter_semantics::UsageMeterSemanticsV1>(hook.clone());
        hook
    }
}
/// Interleave a committed catalog write while the meter is answering outside Pricing's tx.
pub struct MeterHook {
    dsn: String,
    sql: Vec<String>,
    once: std::sync::atomic::AtomicBool,
    repeat: bool,
    clock: Arc<FixedClock>,
    advance: Option<time::Duration>,
    pub provider: p::entry_support::policy_support::MeterProvider,
}
#[async_trait::async_trait]
impl bss_pricing_sdk::meter_semantics::UsageMeterSemanticsV1 for MeterHook {
    async fn resolve(
        &self,
        ctx: &SecurityContext,
        meter: bss_pricing_sdk::terms::MeterRef,
    ) -> Result<
        bss_pricing_sdk::meter_semantics::MeterSemantics,
        toolkit_canonical_errors::CanonicalError,
    > {
        if self.repeat || !self.once.swap(true, std::sync::atomic::Ordering::SeqCst) {
            use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
            let db = Database::connect(&self.dsn).await.unwrap();
            for sql in &self.sql {
                db.execute_raw(Statement::from_string(DbBackend::Sqlite, sql))
                    .await
                    .unwrap();
            }
            if let Some(by) = self.advance {
                self.clock.advance(by);
            }
        }
        self.provider.resolve(ctx, meter).await
    }
}
