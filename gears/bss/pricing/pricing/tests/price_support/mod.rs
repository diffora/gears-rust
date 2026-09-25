//! Shared real REST and database fixture for reference execution.
#![allow(dead_code)]
#![allow(clippy::expect_used, clippy::unwrap_used)]
use axum::{Router, body::Body, http::Request};
use serde_json::{Value, json};
use std::sync::Arc;
use toolkit_security::SecurityContext;
use tower::ServiceExt;
use uuid::Uuid;
#[path = "../storage_support/mod.rs"]
mod storage_support;
struct Resolver {
    tenant: Uuid,
    allow: bool,
}
#[async_trait::async_trait]
impl authz_resolver_sdk::AuthZResolverApi for Resolver {
    async fn evaluate(
        &self,
        _: toolkit_security::PlatformSecurityContext,
        request: authz_resolver_sdk::EvaluationRequest,
    ) -> Result<authz_resolver_sdk::EvaluationResponse, toolkit_canonical_errors::CanonicalError>
    {
        use authz_resolver_sdk::*;
        Ok(EvaluationResponse {
            decision: self.allow
                && request
                    .subject
                    .subject_type
                    .as_deref()
                    .is_some_and(|grant| {
                        grant == "user"
                            || grant
                                == format!(
                                    "{}:{}",
                                    request
                                        .resource
                                        .resource_type
                                        .trim_start_matches("gts.cf.bss.pricing.")
                                        .trim_end_matches(".v1~"),
                                    request.action.name
                                )
                    }),
            context: EvaluationResponseContext {
                constraints: vec![Constraint {
                    predicates: vec![Predicate::In(InPredicate::new(
                        toolkit_security::pep_properties::OWNER_TENANT_ID,
                        vec![self.tenant],
                    ))],
                }],
                deny_reason: None,
            },
        })
    }
}
pub struct Fixture {
    pub app: Router,
    pub denied: Router,
    pub ctx: SecurityContext,
    pub db: toolkit_db::DBProvider<toolkit_db::DbError>,
}
impl Fixture {
    pub async fn new(registry: Arc<dyn bss_products_sdk::ReferenceRegistryV1>) -> Self {
        let (db, _, tenant, _) = storage_support::test_db().await;
        let hub = Arc::new(toolkit::ClientHub::default());
        hub.register::<bss_products_sdk::PricingReferenceRegistry>(Arc::new(
            bss_products_sdk::PricingReferenceRegistry(registry),
        ));
        let state = Arc::new(bss_pricing::api::rest::authoring::AuthoringState {
            db: db.clone(),
            hub,
        });
        let make = |allow| {
            bss_pricing::api::rest::authoring::router(
                state.clone(),
                &toolkit::api::OpenApiRegistryImpl::new(),
            )
            .layer(axum::Extension(authz_resolver_sdk::PolicyEnforcer::new(
                Arc::new(Resolver { tenant, allow }),
            )))
        };
        let ctx = SecurityContext::builder()
            .subject_id(Uuid::new_v4())
            .subject_tenant_id(tenant)
            .subject_type("user")
            .build()
            .unwrap();
        Self {
            app: make(true),
            denied: make(false),
            ctx,
            db,
        }
    }
    pub async fn call(
        &self,
        method: &str,
        path: &str,
        body: Value,
        tag: Option<&str>,
        key: Option<&str>,
    ) -> (u16, Value, String) {
        request(&self.app, &self.ctx, method, path, body, tag, key).await
    }
    pub async fn book(&self) -> (Value, String) {
        let (s, b, t) = self
            .call(
                "POST",
                "/price-books",
                json!({"code":"standard","name":"Standard","currency":"EUR"}),
                None,
                Some("create"),
            )
            .await;
        assert_eq!(s, 201, "{b}");
        (b, t)
    }
}
async fn request(
    app: &Router,
    ctx: &SecurityContext,
    method: &str,
    path: &str,
    body: Value,
    tag: Option<&str>,
    key: Option<&str>,
) -> (u16, Value, String) {
    let mut req = Request::builder()
        .method(method)
        .uri(format!("/bss-pricing/v1{path}"))
        .extension(ctx.clone())
        .header("content-type", "application/json");
    if let Some(tag) = tag {
        req = req.header("if-match", tag);
    }
    if let Some(key) = key {
        req = req.header("idempotency-key", key);
    }
    let response = app
        .clone()
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    let tag = response
        .headers()
        .get("etag")
        .map_or("", |v| v.to_str().unwrap())
        .to_owned();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(json!(null)),
        tag,
    )
}
use bss_products_sdk::{
    ReferenceRegistryV1,
    models::{
        Lifecycle, ReferenceKind, ReferenceState, ReservationReceipt, Sku, SkuType, SkuVersion,
    },
};
use std::sync::atomic::{AtomicUsize, Ordering};
use toolkit_canonical_errors::CanonicalError;
#[derive(Default)]
pub struct Script {
    pub reserve_calls: AtomicUsize,
    pub confirm_calls: AtomicUsize,
    pub releases: AtomicUsize,
    pub mode: AtomicUsize,
    pub parked: tokio::sync::Notify,
    pub resume: tokio::sync::Notify,
    pub refs: tokio::sync::Mutex<std::collections::BTreeMap<Uuid, (Uuid, ReferenceState)>>,
}
impl Script {
    pub fn set(&self, mode: usize) {
        self.mode.store(mode, Ordering::SeqCst);
    }
    pub fn count(value: &AtomicUsize) -> usize {
        value.load(Ordering::SeqCst)
    }
    async fn park(&self) {
        self.parked.notify_one();
        self.resume.notified().await;
    }
}
#[toolkit_canonical_errors::resource_error(toolkit_gts::gts_id!("cf.bss.pricing.price.v1~"))]
struct TestResource;
pub fn refusal(code: &str) -> CanonicalError {
    TestResource::aborted(code).with_reason(code).create()
}
#[async_trait::async_trait]
impl ReferenceRegistryV1 for Script {
    async fn reserve(
        &self,
        _: &SecurityContext,
        _: Uuid,
        _: Uuid,
        _: ReferenceKind,
        ref_id: Uuid,
    ) -> Result<ReservationReceipt, CanonicalError> {
        self.reserve_calls.fetch_add(1, Ordering::SeqCst);
        let mode = self.mode.load(Ordering::SeqCst);
        if mode == 1 {
            self.park().await;
        }
        if mode == 4 {
            return Err(refusal("SKU_FENCED"));
        }
        let mut refs = self.refs.lock().await;
        let entry = refs
            .entry(ref_id)
            .or_insert_with(|| (Uuid::new_v4(), ReferenceState::Reserved));
        if entry.1 == ReferenceState::Released {
            *entry = (Uuid::new_v4(), ReferenceState::Reserved);
        }
        let id = entry.0;
        if mode == 5 {
            self.set(0);
            return Err(CanonicalError::service_unavailable().create());
        }
        Ok(ReservationReceipt {
            reservation_id: id,
            state: entry.1,
        })
    }
    async fn confirm(&self, _: &SecurityContext, _: Uuid, id: Uuid) -> Result<(), CanonicalError> {
        self.confirm_calls.fetch_add(1, Ordering::SeqCst);
        let mode = self.mode.load(Ordering::SeqCst);
        if mode == 2 {
            self.park().await;
        }
        if mode == 6 {
            return Err(CanonicalError::service_unavailable().create());
        }
        if mode == 7 {
            return Err(refusal("REFERENCE_RELEASED"));
        }
        for item in self.refs.lock().await.values_mut() {
            if item.0 == id {
                item.1 = ReferenceState::Confirmed;
            }
        }
        Ok(())
    }
    async fn release(&self, _: &SecurityContext, _: Uuid, id: Uuid) -> Result<(), CanonicalError> {
        if self.mode.load(Ordering::SeqCst) == 3 {
            self.park().await;
        }
        if self.mode.load(Ordering::SeqCst) == 12 {
            return Err(CanonicalError::service_unavailable().create());
        }
        self.releases.fetch_add(1, Ordering::SeqCst);
        for item in self.refs.lock().await.values_mut() {
            if item.0 == id {
                item.1 = ReferenceState::Released;
            }
        }
        Ok(())
    }
    async fn states(
        &self,
        _: &SecurityContext,
        _: Uuid,
        ids: &[Uuid],
    ) -> Result<Vec<(Uuid, ReferenceState)>, CanonicalError> {
        let refs = self.refs.lock().await;
        Ok(ids
            .iter()
            .map(|id| (*id, refs.values().find(|v| v.0 == *id).unwrap().1))
            .collect())
    }
    async fn sku_for_write(
        &self,
        _: &SecurityContext,
        tenant: Uuid,
        id: Uuid,
    ) -> Result<Sku, CanonicalError> {
        let mode = self.mode.load(Ordering::SeqCst);
        Ok(Sku {
            id,
            tenant_id: tenant,
            code: "cpu".into(),
            name: "CPU".into(),
            r#type: if mode == 8 {
                SkuType::Bundle
            } else if mode == 11 {
                SkuType::Recurring
            } else {
                SkuType::Usage
            },
            category_id: Uuid::new_v4(),
            description: String::new(),
            sellable: true,
            lifecycle: if mode == 9 {
                Lifecycle::Deprecated
            } else if mode == 10 {
                Lifecycle::Draft
            } else {
                Lifecycle::Published
            },
            revision: 1,
            published_version: 1,
            gl_code: None,
            tax_category: None,
            invoice_line_template: None,
            billing_timing: None,
            usage_type_ref: None,
            unit: None,
            type_change_pending: false,
            pending_unit_id: None,
            approved_by_unit_id: None,
            created_by: Uuid::new_v4(),
            created_at: time::OffsetDateTime::now_utc(),
            updated_at: time::OffsetDateTime::now_utc(),
        })
    }
    async fn sku_version_as_of(
        &self,
        _: &SecurityContext,
        _: Uuid,
        _: Uuid,
        _: time::Date,
    ) -> Result<Option<SkuVersion>, CanonicalError> {
        Ok(None)
    }
}

use bss_pricing::infra::storage::entity::{price, price_row};
use storage_support::at;
pub fn row(p: &price::Model) -> price_row::Model {
    price_row::Model {
        id: Uuid::new_v4(),
        tenant_id: p.tenant_id,
        price_id: p.id,
        version_no: 1,
        dim_value: None,
        model: "per_unit".into(),
        price_json: serde_json::json!({"rate":"0.1"}),
        min_fee: Some(rust_decimal::Decimal::new(1234, 2)),
        eligibility: "all".into(),
        effective_from: at(9).date(),
        effective_to: None,
        keep_for_bound: false,
        closed_explicitly: false,
        temporary_until: None,
        paired_row_id: None,
        return_of_row_id: None,
        state: "draft".into(),
        pending_unit_id: None,
        approved_by_unit_id: None,
        note: None,
        created_by: Uuid::new_v4(),
        approved_at: None,
        version: 1,
        created_at: at(9),
        updated_at: at(9),
    }
}
