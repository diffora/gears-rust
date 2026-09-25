//! Pricing-only boot harness. Platform clients are doubles; no products is registered.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use async_trait::async_trait;
use axum::Router;
use bss_pricing::module::BssPricingGear;
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use toolkit::api::OpenApiRegistryImpl;
use toolkit::contracts::RestApiCapability;
use toolkit::{Gear, GearCtx};
use toolkit_canonical_errors::CanonicalError;
use types_registry_sdk::{
    GtsInstance, GtsTypeSchema, InstanceQuery, RegisterResult, TypeSchemaQuery, TypesRegistryClient,
};
use uuid::Uuid;

#[path = "../common/mod.rs"]
pub mod common;

struct Config(serde_json::Value);
impl toolkit::config::ConfigProvider for Config {
    fn get_gear_config(&self, name: &str) -> Option<&serde_json::Value> {
        (name == "bss-pricing").then_some(&self.0)
    }
}

struct DenyingResolver;
#[async_trait]
impl authz_resolver_sdk::AuthZResolverApi for DenyingResolver {
    async fn evaluate(
        &self,
        _ctx: toolkit_security::PlatformSecurityContext,
        _request: authz_resolver_sdk::EvaluationRequest,
    ) -> Result<authz_resolver_sdk::EvaluationResponse, CanonicalError> {
        Err(CanonicalError::service_unavailable().create())
    }
}

/// Captures the mandatory authz schema registration, even for the empty frame.
#[derive(Default)]
pub struct Registry {
    pub calls: AtomicUsize,
}
#[async_trait]
impl TypesRegistryClient for Registry {
    async fn register(
        &self,
        entities: Vec<serde_json::Value>,
    ) -> Result<Vec<RegisterResult>, CanonicalError> {
        assert_eq!(entities, bss_pricing::authz::authz_label_type_schemas());
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Vec::new())
    }
    async fn register_type_schemas(
        &self,
        _type_schemas: Vec<serde_json::Value>,
    ) -> Result<Vec<RegisterResult>, CanonicalError> {
        Err(CanonicalError::internal("unexpected registry call").create())
    }
    async fn get_type_schema(&self, _type_id: &str) -> Result<GtsTypeSchema, CanonicalError> {
        Err(CanonicalError::internal("unexpected registry call").create())
    }
    async fn get_type_schema_by_uuid(
        &self,
        _type_uuid: Uuid,
    ) -> Result<GtsTypeSchema, CanonicalError> {
        Err(CanonicalError::internal("unexpected registry call").create())
    }
    async fn get_type_schemas(
        &self,
        _type_ids: Vec<String>,
    ) -> HashMap<String, Result<GtsTypeSchema, CanonicalError>> {
        HashMap::new()
    }
    async fn get_type_schemas_by_uuid(
        &self,
        _type_uuids: Vec<Uuid>,
    ) -> HashMap<Uuid, Result<GtsTypeSchema, CanonicalError>> {
        HashMap::new()
    }
    async fn list_type_schemas(
        &self,
        _query: TypeSchemaQuery,
    ) -> Result<Vec<GtsTypeSchema>, CanonicalError> {
        Err(CanonicalError::internal("unexpected registry call").create())
    }
    async fn register_instances(
        &self,
        _instances: Vec<serde_json::Value>,
    ) -> Result<Vec<RegisterResult>, CanonicalError> {
        Err(CanonicalError::internal("unexpected registry call").create())
    }
    async fn get_instance(&self, _id: &str) -> Result<GtsInstance, CanonicalError> {
        Err(CanonicalError::internal("unexpected registry call").create())
    }
    async fn get_instance_by_uuid(&self, _uuid: Uuid) -> Result<GtsInstance, CanonicalError> {
        Err(CanonicalError::internal("unexpected registry call").create())
    }
    async fn get_instances(
        &self,
        _ids: Vec<String>,
    ) -> HashMap<String, Result<GtsInstance, CanonicalError>> {
        HashMap::new()
    }
    async fn get_instances_by_uuid(
        &self,
        _uuids: Vec<Uuid>,
    ) -> HashMap<Uuid, Result<GtsInstance, CanonicalError>> {
        HashMap::new()
    }
    async fn list_instances(
        &self,
        _query: InstanceQuery,
    ) -> Result<Vec<GtsInstance>, CanonicalError> {
        Err(CanonicalError::internal("unexpected registry call").create())
    }
}

/// An initialized pricing gear and its real configuration/database context.
pub struct Harness {
    pub gear: BssPricingGear,
    pub ctx: GearCtx,
    pub registry: Arc<Registry>,
}

impl Harness {
    /// Initialize pricing with platform clients only.
    ///
    /// # Errors
    /// Propagates migration and initialization failures.
    pub async fn new() -> anyhow::Result<Self> {
        let db = common::migrated_db().await?;
        let hub = Arc::new(toolkit::ClientHub::new());
        hub.register::<dyn authz_resolver_sdk::AuthZResolverApi>(Arc::new(DenyingResolver));
        let registry = Arc::new(Registry::default());
        hub.register::<dyn TypesRegistryClient>(registry.clone());
        let ctx = GearCtx::new(
            "bss-pricing",
            Uuid::new_v4(),
            Arc::new(Config(serde_json::json!({"config": {}}))),
            hub,
            tokio_util::sync::CancellationToken::new(),
        )
        .with_db(db);
        let gear = BssPricingGear::default();
        gear.init(&ctx).await?;
        Ok(Self {
            gear,
            ctx,
            registry,
        })
    }

    /// Register through the actual gear capability, preserving the host router.
    ///
    /// # Errors
    /// Propagates router registration failures.
    /// # Panics
    /// Fails if the runtime route inventory differs from the exact census.
    pub fn router(&self, host: Router) -> anyhow::Result<(Router, OpenApiRegistryImpl)> {
        let registry = OpenApiRegistryImpl::new();
        let router = self.gear.register_rest(&self.ctx, host, &registry)?;
        let registered: std::collections::BTreeSet<_> = registry
            .operation_specs
            .iter()
            .map(|e| {
                let (m, p) = e.key().split_once(':').unwrap();
                (m.to_owned(), p.to_owned())
            })
            .collect();
        let expected: std::collections::BTreeSet<_> = [
            ("POST", "/bss-pricing/v1/price-books"),
            ("POST", "/bss-pricing/v1/price-books/{id}/prices"),
            ("GET", "/bss-pricing/v1/prices/{id}"),
            ("PATCH", "/bss-pricing/v1/prices/{id}"),
            ("DELETE", "/bss-pricing/v1/prices/{id}"),
            ("GET", "/bss-pricing/v1/reference-ops"),
            ("GET", "/bss-pricing/v1/price-books"),
            ("GET", "/bss-pricing/v1/price-books/{id}"),
            ("PATCH", "/bss-pricing/v1/price-books/{id}"),
            ("GET", "/bss-pricing/v1/price-books/{id}/prices"),
            ("GET", "/bss-pricing/v1/price-books/{id}/export"),
            ("GET", "/bss-pricing/v1/settings"),
            ("PUT", "/bss-pricing/v1/settings"),
            ("GET", "/bss-pricing/v1/dimension-keys"),
            ("PUT", "/bss-pricing/v1/dimension-keys"),
            ("POST", "/bss-pricing/v1/prices/{id}/rows"),
            ("PATCH", "/bss-pricing/v1/rows/{id}"),
            ("DELETE", "/bss-pricing/v1/rows/{id}"),
            ("POST", "/bss-pricing/v1/rows/{id}/submit"),
            ("GET", "/bss-pricing/v1/price-books/{id}/publish-changes"),
            ("POST", "/bss-pricing/v1/price-books/{id}/publish-changes"),
            ("GET", "/bss-pricing/v1/approval-units"),
            ("GET", "/bss-pricing/v1/approval-units/{id}"),
            ("POST", "/bss-pricing/v1/approval-units/{id}/approve"),
            ("POST", "/bss-pricing/v1/approval-units/{id}/reject"),
            ("POST", "/bss-pricing/v1/approval-units/{id}/withdraw"),
            ("GET", "/bss-pricing/v1/approval-policy"),
            ("PUT", "/bss-pricing/v1/approval-policy"),
        ]
        .into_iter()
        .map(|(m, p)| (m.to_owned(), p.to_owned()))
        .collect();
        assert_eq!(registered, expected);
        Ok((router, registry))
    }
}

// Run-3 route contract: method | path | resource:action | If-Match | Idempotency-Key
// POST /price-books price_book:author false true
// GET /price-books price_book:read false false
// GET /price-books/{id} price_book:read false false
// PATCH /price-books/{id} price_book:author true false
// GET /price-books/{id}/prices price:read false false
// GET /price-books/{id}/export price_book:read false false
// GET /settings config:read false false
// PUT /settings config:settings true false
// GET /dimension-keys config:read false false
// PUT /dimension-keys config:settings true false

// POST /price-books/{id}/prices price:author false true
// GET /prices/{id} price:read false false
// PATCH /prices/{id} price:author true false
// DELETE /prices/{id} price:author false false

// GET /reference-ops config:settings false false

// Run-4 rows: method | path | resource:action | If-Match | Idempotency-Key
// POST /prices/{id}/rows price:author false true
// PATCH /rows/{id} price:author true false
// DELETE /rows/{id} price:author true false

// Run-4 approvals: method | path | resource:action | If-Match | Idempotency-Key
// POST /rows/{id}/submit price:submit false true
// GET /price-books/{id}/publish-changes price_book:read false false
// POST /price-books/{id}/publish-changes price_book:submit false true
// GET /approval-units approval_unit:read false false
// GET /approval-units/{id} approval_unit:read false false
// POST /approval-units/{id}/approve approval_unit:approve false true
// POST /approval-units/{id}/reject approval_unit:approve false true
// POST /approval-units/{id}/withdraw approval_unit:submit false true
// GET /approval-policy config:read false false
// PUT /approval-policy config:settings true false
