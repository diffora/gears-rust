//! The in-process bindings of the SDK's client traits (`design/12` §2.4
//! `inst-sdk-surface`, P-D-15, P-D-151): the read-model client, the
//! authoring/publish client, the freeze-acknowledgment client with its release
//! half and the bundle composition-completed signal. The increment-request
//! and watermark bindings live beside their doors
//! (`catalog_version::InProcessIncrementRequests`,
//! `reference::InProcessWatermarkPosts`); these four are gathered here because
//! they share one shape.
//!
//! # The binding runs the door
//!
//! Every write binding calls **the door's own handler function** with the
//! caller's context, the typed headers and the typed body, and reads the
//! answer the door renders. That is the whole point of the in-process
//! deployment (P-D-15): the SDK write and the REST write are one door, so the
//! same idempotency key is one key, the same `If-Match` is one precondition,
//! the same gate is one gate and the same audit row is written. Nothing is
//! re-implemented here, so nothing can drift from the REST binding.
//!
//! The read binding goes to the repository under the same `× read` grant the
//! GET doors spend, because the SDK's read shape carries `composition_pending`
//! (`dod-catalogsku-shape`) and the REST view does not.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-sdk-surface:p1

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::Path;
use axum::http::header::{ETAG, IF_MATCH};
use axum::http::{HeaderMap, HeaderValue};
use axum::response::Response;
use axum::{Extension, Json};
use bss_products_sdk::ProductsClient;
use bss_products_sdk::authoring::{
    Authoring, FieldValue, HeadReceipt, NewProduct, NewSku, Precondition, SaveFields,
};
use bss_products_sdk::composition::{CompositionOutcome, CompositionSignals};
use bss_products_sdk::freeze::{FreezeAcks, FreezeEdgeReceipt};
use bss_products_sdk::models::{LifecycleState, Product, Sku, SkuType};
use serde_json::Value;
use toolkit::api::canonical_prelude::{CanonicalError, resource_error};
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::rest::dto::FreezeParticipantRequest;
use crate::api::rest::{
    ApiState, IDEMPOTENCY_KEY_HEADER, catalog_version, preconditions, products, skus,
};
use crate::domain::concurrency::InternalRevision;
use crate::infra::storage::repo;

/// The canonical-error identity of this port's own refusals.
#[resource_error(gts_id!("cf.bss.products.product.v1~"))]
struct SdkResource;

/// The state every binding holds: the door's own state and the PEP.
#[derive(Clone)]
pub(crate) struct Binding {
    /// The door's state: database, outbox sink, knobs.
    pub(crate) state: Arc<ApiState>,
    /// The platform PEP, the same instance the routers layer.
    pub(crate) enforcer: authz_resolver_sdk::PolicyEnforcer,
}

impl Binding {
    // The handlers take the extractor as `Option` (an unauthenticated call
    // is `None`); the binding always has a caller.
    #[allow(clippy::unnecessary_wraps)]
    fn ctx(ctx: &SecurityContext) -> Option<Extension<SecurityContext>> {
        Some(Extension(ctx.clone()))
    }
}

fn internal(what: &str, detail: impl std::fmt::Display) -> CanonicalError {
    CanonicalError::internal(format!("bss-products sdk binding: {what}: {detail}")).create()
}

/// The typed preconditions as the headers the door reads.
fn headers_of(precondition: &Precondition) -> Result<HeaderMap, CanonicalError> {
    let mut headers = HeaderMap::new();
    if let Some(revision) = precondition.if_match {
        let tag = preconditions::etag(InternalRevision::new(revision));
        headers.insert(
            IF_MATCH,
            HeaderValue::from_str(&tag).map_err(|e| internal("If-Match", e))?,
        );
    }
    if let Some(key) = &precondition.idempotency_key {
        headers.insert(
            IDEMPOTENCY_KEY_HEADER,
            HeaderValue::from_str(key).map_err(|e| internal("Idempotency-Key", e))?,
        );
    }
    Ok(headers)
}

/// The door's answer as JSON, with whether it carried an `ETag` — a replayed
/// answer never does (`replay_response`'s own doc).
async fn answer_of(response: Response) -> Result<(bool, Value), CanonicalError> {
    let fresh = response.headers().contains_key(ETAG);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .map_err(|e| internal("reading the door's answer", e))?;
    let body: Value =
        serde_json::from_slice(&bytes).map_err(|e| internal("the door's answer is not JSON", e))?;
    Ok((fresh, body))
}

/// The doors render two spellings: the `api_dto` views (`GET`, create) are
/// `snake_case`, the act bodies rendered inside the head-act transactions
/// (save, publish, discard) are `camelCase`. The binding reads either, and the
/// divergence is filed (`features/consumer-contracts.md` §7, P-D-151) rather
/// than papered over here.
fn field<'a>(body: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().map(|k| &body[*k]).find(|v| !v.is_null())
}

fn field_i64(body: &Value, keys: &[&str]) -> Result<i64, CanonicalError> {
    field(body, keys)
        .and_then(Value::as_i64)
        .ok_or_else(|| internal("the door's answer", format!("`{}` is missing", keys[0])))
}

fn field_uuid(body: &Value, keys: &[&str]) -> Result<Uuid, CanonicalError> {
    field(body, keys)
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| internal("the door's answer", format!("`{}` is not an id", keys[0])))
}

fn field_str<'a>(body: &'a Value, keys: &[&str]) -> Result<&'a str, CanonicalError> {
    field(body, keys)
        .and_then(Value::as_str)
        .ok_or_else(|| internal("the door's answer", format!("`{}` is missing", keys[0])))
}

async fn receipt_of(response: Response, id_keys: &[&str]) -> Result<HeadReceipt, CanonicalError> {
    let (fresh, body) = answer_of(response).await?;
    let state = field_str(&body, &["lifecycleState", "lifecycle_state"])?;
    Ok(HeadReceipt {
        entity_id: field_uuid(&body, id_keys)?,
        internal_revision: field_i64(&body, &["internalRevision", "internal_revision"])?,
        lifecycle_state: LifecycleState::parse(state)
            .ok_or_else(|| internal("the door's answer", format!("state `{state}`")))?,
        published_version: field_i64(&body, &["publishedVersion", "published_version"])?,
        replayed: !fresh,
    })
}

fn json_fields(fields: SaveFields) -> BTreeMap<String, Value> {
    fields
        .into_iter()
        .map(|(key, value)| {
            let value = match value {
                FieldValue::Text(s) => Value::String(s),
                FieldValue::Bool(b) => Value::Bool(b),
                FieldValue::Integer(i) => Value::from(i),
                FieldValue::Null => Value::Null,
            };
            (key, value)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The authoring/publish client
// ---------------------------------------------------------------------------

/// The in-process [`Authoring`] binding: six doors, called as themselves.
pub(crate) struct InProcessAuthoring(pub(crate) Binding);

#[async_trait]
impl Authoring for InProcessAuthoring {
    async fn create_product(
        &self,
        ctx: &SecurityContext,
        product: NewProduct,
        precondition: Precondition,
    ) -> Result<HeadReceipt, CanonicalError> {
        let response = products::create_product(
            Extension(Arc::clone(&self.0.state)),
            Extension(self.0.enforcer.clone()),
            Binding::ctx(ctx),
            headers_of(&precondition)?,
            Json(products::CreateProductRequest {
                id: product.id,
                brand_id: product.brand_id,
                name: product.name,
                product_code: product.product_code,
                region_scope: product.region_scope,
                brand_scope: product.brand_scope,
            }),
        )
        .await?;
        receipt_of(response, &["productId", "product_id"]).await
    }

    async fn save_product(
        &self,
        ctx: &SecurityContext,
        product_id: Uuid,
        fields: SaveFields,
        precondition: Precondition,
    ) -> Result<HeadReceipt, CanonicalError> {
        let response = products::save_product(
            Extension(Arc::clone(&self.0.state)),
            Extension(self.0.enforcer.clone()),
            Binding::ctx(ctx),
            Path(product_id),
            headers_of(&precondition)?,
            Json(products::SaveProductRequest {
                fields: json_fields(fields),
            }),
        )
        .await?;
        receipt_of(response, &["productId", "product_id"]).await
    }

    async fn publish_product(
        &self,
        ctx: &SecurityContext,
        product_id: Uuid,
        precondition: Precondition,
    ) -> Result<HeadReceipt, CanonicalError> {
        let response = products::publish_product(
            Extension(Arc::clone(&self.0.state)),
            Extension(self.0.enforcer.clone()),
            Binding::ctx(ctx),
            Path(product_id),
            headers_of(&precondition)?,
        )
        .await?;
        receipt_of(response, &["productId", "product_id"]).await
    }

    async fn create_sku(
        &self,
        ctx: &SecurityContext,
        sku: NewSku,
        precondition: Precondition,
    ) -> Result<HeadReceipt, CanonicalError> {
        let response = skus::create_sku(
            Extension(Arc::clone(&self.0.state)),
            Extension(self.0.enforcer.clone()),
            Binding::ctx(ctx),
            headers_of(&precondition)?,
            Json(skus::CreateSkuRequest {
                id: sku.id,
                product_id: sku.product_id,
                sku_code: sku.sku_code,
                region_scope: sku.region_scope,
                brand_scope: sku.brand_scope,
                sku_type: sku.sku_type,
                sellable: sku.sellable,
                plan_tier: sku.plan_tier,
                tax_category_ref: sku.tax_category_ref,
                gl_code_ref: sku.gl_code_ref,
            }),
        )
        .await?;
        receipt_of(response, &["skuId", "sku_id"]).await
    }

    async fn save_sku(
        &self,
        ctx: &SecurityContext,
        sku_id: Uuid,
        fields: SaveFields,
        precondition: Precondition,
    ) -> Result<HeadReceipt, CanonicalError> {
        let response = skus::save_sku(
            Extension(Arc::clone(&self.0.state)),
            Extension(self.0.enforcer.clone()),
            Binding::ctx(ctx),
            headers_of(&precondition)?,
            Path(sku_id),
            Json(skus::SaveSkuRequest {
                fields: json_fields(fields),
            }),
        )
        .await?;
        receipt_of(response, &["skuId", "sku_id"]).await
    }

    async fn publish_sku(
        &self,
        ctx: &SecurityContext,
        sku_id: Uuid,
        precondition: Precondition,
    ) -> Result<HeadReceipt, CanonicalError> {
        let response = skus::publish_sku(
            Extension(Arc::clone(&self.0.state)),
            Extension(self.0.enforcer.clone()),
            Binding::ctx(ctx),
            headers_of(&precondition)?,
            Path(sku_id),
        )
        .await?;
        receipt_of(response, &["skuId", "sku_id"]).await
    }
}

// ---------------------------------------------------------------------------
// The freeze-acknowledgment client, with its release half
// ---------------------------------------------------------------------------

/// The in-process [`FreezeAcks`] binding over `design/06`'s two participant
/// doors.
pub(crate) struct InProcessFreezeAcks(pub(crate) Binding);

async fn edge_receipt(response: Response) -> Result<FreezeEdgeReceipt, CanonicalError> {
    let (_, body) = answer_of(response).await?;
    Ok(FreezeEdgeReceipt {
        participant: field_str(&body, &["participant"])?.to_owned(),
        state: field_str(&body, &["state"])?.to_owned(),
        changed: body["changed"].as_bool().unwrap_or(false),
    })
}

#[async_trait]
impl FreezeAcks for InProcessFreezeAcks {
    async fn ack(
        &self,
        ctx: &SecurityContext,
        catalog_version_id: i64,
        participant: &str,
    ) -> Result<FreezeEdgeReceipt, CanonicalError> {
        let response = catalog_version::ack_catalog_version(
            Extension(Arc::clone(&self.0.state)),
            Extension(self.0.enforcer.clone()),
            Binding::ctx(ctx),
            Path(catalog_version_id),
            Json(FreezeParticipantRequest {
                participant: participant.to_owned(),
            }),
        )
        .await?;
        edge_receipt(response).await
    }

    async fn release(
        &self,
        ctx: &SecurityContext,
        catalog_version_id: i64,
        participant: &str,
    ) -> Result<FreezeEdgeReceipt, CanonicalError> {
        let response = catalog_version::release_catalog_version(
            Extension(Arc::clone(&self.0.state)),
            Extension(self.0.enforcer.clone()),
            Binding::ctx(ctx),
            Path(catalog_version_id),
            Json(FreezeParticipantRequest {
                participant: participant.to_owned(),
            }),
        )
        .await?;
        edge_receipt(response).await
    }
}

// ---------------------------------------------------------------------------
// The bundle composition-completed signal
// ---------------------------------------------------------------------------

/// The in-process [`CompositionSignals`] binding over the composition-clear
/// door. A signal that had nothing to clear is `nothing` (P-D-159), a re-sent one `replayed`;
/// (one answer for "this ran" and "this had nothing to do"), and the binding
/// carries that fold rather than inventing a distinction the door does not
/// make.
pub(crate) struct InProcessCompositionSignals(pub(crate) Binding);

#[async_trait]
impl CompositionSignals for InProcessCompositionSignals {
    async fn composed(
        &self,
        ctx: &SecurityContext,
        sku_id: Uuid,
        signal_ref: Uuid,
    ) -> Result<CompositionOutcome, CanonicalError> {
        let response = skus::clear_composition(
            Extension(Arc::clone(&self.0.state)),
            Extension(self.0.enforcer.clone()),
            Binding::ctx(ctx),
            Path(sku_id),
            Json(skus::CompositionClearRequest { signal_ref }),
        )
        .await?;
        let (_, body) = answer_of(response).await?;
        match field_str(&body, &["outcome"])? {
            "cleared" => Ok(CompositionOutcome::Cleared {
                published_version: field_i64(&body, &["publishedVersion", "published_version"])?,
            }),
            "held" => Ok(CompositionOutcome::Held {
                on: field(&body, &["heldOn", "held_on"])
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_owned(),
            }),
            "replayed" => Ok(CompositionOutcome::Replayed),
            "nothing" => Ok(CompositionOutcome::Nothing),
            other => Err(internal("the door's answer", format!("outcome `{other}`"))),
        }
    }
}

// ---------------------------------------------------------------------------
// The read-model client
// ---------------------------------------------------------------------------

/// The in-process [`ProductsClient`] binding: the `× read` grant the GET
/// doors spend, then the repository — because the SDK read shape carries
/// `composition_pending` and the REST view does not.
pub(crate) struct InProcessProductsClient(pub(crate) Binding);

impl InProcessProductsClient {
    async fn read_scope(
        &self,
        ctx: &SecurityContext,
        resource: &authz_resolver_sdk::pep::ResourceType,
        tenant_id: Uuid,
    ) -> Result<toolkit_db::secure::AccessScope, CanonicalError> {
        crate::authz::access_scope(
            &self.0.enforcer,
            ctx,
            resource,
            crate::authz::actions::READ,
            Some(tenant_id),
            None,
            true,
        )
        .await
        .map_err(|e| {
            crate::api::rest::authz_error_to_canonical(e, |reason| {
                SdkResource::permission_denied()
                    .with_reason(reason)
                    .create()
            })
        })
    }

    fn conn(&self) -> Result<toolkit_db::DbConn<'_>, CanonicalError> {
        self.0.state.db.conn().map_err(|e| internal("db conn", e))
    }
}

fn not_found(entity_id: Uuid) -> CanonicalError {
    SdkResource::not_found("no entity matches this id in the caller's scope")
        .with_resource(entity_id.to_string())
        .create()
}

#[async_trait]
impl ProductsClient for InProcessProductsClient {
    async fn get_product(
        &self,
        ctx: &SecurityContext,
        tenant_id: Uuid,
        product_id: Uuid,
    ) -> Result<Product, CanonicalError> {
        let scope = self
            .read_scope(ctx, &crate::authz::resource_types::PRODUCT, tenant_id)
            .await?;
        let conn = self.conn()?;
        let record = repo::find_product(&conn, &scope, tenant_id, product_id)
            .await
            .map_err(|e| crate::api::rest::repo_error_to_canonical(&e))?
            .ok_or_else(|| not_found(product_id))?;
        Ok(Product {
            product_id: record.product_id,
            tenant_id: record.tenant_id,
            brand_id: record.brand_id,
            name: record.name,
            product_code: record.product_code,
            lifecycle_state: record.lifecycle_state,
            internal_revision: record.internal_revision,
            published_version: record.published_version,
        })
    }

    async fn get_sku(
        &self,
        ctx: &SecurityContext,
        tenant_id: Uuid,
        sku_id: Uuid,
    ) -> Result<Sku, CanonicalError> {
        let scope = self
            .read_scope(ctx, &crate::authz::resource_types::SKU, tenant_id)
            .await?;
        let conn = self.conn()?;
        let record = repo::find_sku(&conn, &scope, tenant_id, sku_id)
            .await
            .map_err(|e| crate::api::rest::repo_error_to_canonical(&e))?
            .ok_or_else(|| not_found(sku_id))?;
        // The read shape's `sku_type` is the closed type (`dod-sdk-read-shape`);
        // an untyped draft is not on the shape a consumer reads, and is
        // answered as a consumer's miss rather than as a shape with a hole.
        let sku_type = record
            .sku_type
            .as_deref()
            .and_then(SkuType::parse)
            .ok_or_else(|| not_found(sku_id))?;
        Ok(Sku {
            sku_id: record.sku_id,
            tenant_id: record.tenant_id,
            product_id: record.product_id,
            sku_code: record.sku_code,
            lifecycle_state: record.lifecycle_state,
            internal_revision: record.internal_revision,
            published_version: record.published_version,
            sku_type,
            sellable: record.sellable,
            composition_pending: record.composition_pending,
            plan_tier: record.plan_tier,
            metering_unit: record.metering_unit,
            usage_type_ref: record.usage_type_ref,
            tax_category_ref: record.tax_category_ref,
            gl_code_ref: record.gl_code_ref,
        })
    }
}

#[cfg(test)]
#[path = "sdk_bindings_tests.rs"]
mod sdk_bindings_tests;
