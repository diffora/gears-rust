//! Price writes and their durable registry operations.
//!
//! @cpt-dod:cpt-cf-bss-pricing-dod-price-key-unique:p1
//! @cpt-dod:cpt-cf-bss-pricing-dod-price-metadata:p1
//! @cpt-dod:cpt-cf-bss-pricing-dod-price-reference-handoff:p1
use super::{
    AuthoringState,
    dto::{PricingPriceCreate, PricingPriceDto, PricingPricePatch},
    support::{self, DoorError},
};
use crate::{
    domain::price::{self, OpKind},
    infra::{
        reference_work::{self, Receipt, WallClock, Work},
        storage::{
            entity,
            repo::{book_repo, idempotency_repo as idem, price_repo, reference_op_repo, row_repo},
        },
    },
};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use toolkit_canonical_errors::CanonicalError;
use toolkit_db::secure::{AccessScope, DBRunner};
use toolkit_security::SecurityContext;
use uuid::Uuid;
pub(super) async fn find(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<entity::price::Model, DoorError> {
    price_repo::find(tx, scope, tenant, id)
        .await?
        .ok_or_else(|| support::missing().into())
}
fn validate_template(input: Option<&str>) -> Result<(), CanonicalError> {
    if let Some(template) = input {
        price::validate_template(template)
            .map_err(|e| support::invalid("invoice_line_override", e.code))?;
    }
    Ok(())
}
enum Begun {
    Replay(Receipt),
    Op(Uuid),
}
#[allow(
    clippy::too_many_arguments,
    reason = "authorized door identity and replay operands"
)]
pub(super) async fn create(
    state: Arc<AuthoringState>,
    scope: AccessScope,
    ctx: SecurityContext,
    book: Uuid,
    correlation: Uuid,
    key: String,
    digest: Vec<u8>,
    input: PricingPriceCreate,
) -> Result<Response, CanonicalError> {
    let original_ctx = ctx.clone();
    let result = support::transaction(&state.db.db(), move |tx| {
        let (scope, ctx, key, digest, input) = (
            scope.clone(),
            ctx.clone(),
            key.clone(),
            digest.clone(),
            input.clone(),
        );
        Box::pin(async move {
            let tenant = ctx.subject_tenant_id();
            let now = time::OffsetDateTime::now_utc();
            let receipt_scope = AccessScope::for_tenant(tenant);
            let endpoint = format!("/bss-pricing/v1/price-books/{book}/prices");
            match idem::claim_idempotency_key(
                tx,
                &receipt_scope,
                tenant,
                &endpoint,
                &key,
                &digest,
                now,
                now + time::Duration::hours(24),
            )
            .await?
            {
                idem::IdempotencyClaim::Claimed => {}
                idem::IdempotencyClaim::Answered {
                    payload_hash,
                    response_body,
                    ..
                } => {
                    if payload_hash != digest {
                        return Err(support::conflict("IDEMPOTENCY_CONFLICT").into());
                    }
                    return Ok(Begun::Replay(
                        serde_json::from_value(response_body).map_err(|_| {
                            CanonicalError::internal("invalid price receipt").create()
                        })?,
                    ));
                }
                idem::IdempotencyClaim::InFlight { payload_hash, .. } if payload_hash != digest => {
                    return Err(support::conflict("IDEMPOTENCY_CONFLICT").into());
                }
                _ => return Err(support::conflict("IDEMPOTENCY_KEY_IN_FLIGHT").into()),
            }
            validate_template(input.invoice_line_override.as_deref())?;
            if book_repo::find(tx, &scope, tenant, book).await?.is_none() {
                return Err(support::missing().into());
            }
            let work = Work {
                book_id: book,
                input,
                correlation,
                refusal: None,
                receipt: None,
            };
            let op = reference_work::new_op(
                &ctx,
                Uuid::now_v7(),
                &work,
                OpKind::Create,
                None,
                Some(key.clone()),
                now,
            )?;
            let id = op.op_id;
            reference_op_repo::insert(tx, &receipt_scope, op).await?;
            idem::bind_op(tx, &receipt_scope, tenant, &endpoint, &key, id).await?;
            Ok(Begun::Op(id))
        })
    })
    .await?;
    match result {
        Begun::Replay(receipt) => receipt.response(),
        Begun::Op(id) => reference_work::drive(&state, &original_ctx, id, Arc::new(WallClock))
            .await?
            .ok_or_else(|| CanonicalError::internal("missing create receipt").create())?
            .response(),
    }
}
pub(super) async fn patch(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    id: Uuid,
    version: u64,
    input: PricingPricePatch,
) -> Result<Response, DoorError> {
    let tenant = ctx.subject_tenant_id();
    let mut m = find(tx, scope, tenant, id).await?;
    support::check_version(version, m.version)?;
    if let Some(dimension) = input.dimension_key {
        if dimension != m.dimension_key
            && row_repo::for_price(tx, scope, tenant, id)
                .await?
                .iter()
                .any(|r| r.dim_value.is_some())
        {
            return Err(support::conflict("DIMENSION_KEY_IN_USE").into());
        }
        m.dimension_key = dimension;
    }
    if let Some(template) = input.invoice_line_override {
        validate_template(template.as_deref())?;
        m.invoice_line_override = template;
    }
    m.updated_at = time::OffsetDateTime::now_utc();
    price_repo::update(tx, scope, m.clone()).await?;
    m.version += 1;
    support::audit(tx, ctx, correlation, "price.patch", id, m.version).await?;
    Ok(support::response(
        StatusCode::OK,
        &PricingPriceDto::from(m),
        Some(version + 1),
    )?)
}
pub(super) async fn delete(
    state: Arc<AuthoringState>,
    scope: AccessScope,
    ctx: SecurityContext,
    correlation: Uuid,
    id: Uuid,
) -> Result<Response, CanonicalError> {
    let original_ctx = ctx.clone();
    let op_id = support::transaction(&state.db.db(), move |tx| {
        let (scope, ctx) = (scope.clone(), ctx.clone());
        Box::pin(async move {
            let tenant = ctx.subject_tenant_id();
            let m = find(tx, &scope, tenant, id).await?;
            // A pending create must complete before deletion, otherwise its confirm could lose its price.
            if m.reference_state == "confirmation_pending" {
                return Err(support::conflict("PRICE_CONFIRMATION_PENDING").into());
            }
            for row in row_repo::for_price(tx, &scope, tenant, id).await? {
                if row.state != "draft" || row.pending_unit_id.is_some() {
                    return Err(support::conflict("PRICE_ROWS_IN_USE").into());
                }
                row_repo::delete_draft(tx, &scope, tenant, row.id, row.version).await?;
            }
            let work = Work {
                book_id: m.book_id,
                input: PricingPriceCreate {
                    sku_id: m.sku_id,
                    period: m.period,
                    dimension_key: m.dimension_key,
                    invoice_line_override: m.invoice_line_override,
                },
                correlation,
                refusal: None,
                receipt: None,
            };
            let op = reference_work::new_op(
                &ctx,
                id,
                &work,
                OpKind::Delete,
                Some(m.reservation_id),
                None,
                time::OffsetDateTime::now_utc(),
            )?;
            let op_id = op.op_id;
            price_repo::delete_empty(tx, &scope, tenant, id, m.version).await?;
            reference_op_repo::insert(tx, &AccessScope::for_tenant(tenant), op).await?;
            support::audit(tx, &ctx, correlation, "price.delete", id, m.version).await?;
            Ok(op_id)
        })
    })
    .await?;
    reference_work::drive(&state, &original_ctx, op_id, Arc::new(WallClock)).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
