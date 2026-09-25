//! Book writes share one transaction with their audit and POST receipt.
use super::{
    dto::{PriceBookCreate, PriceBookDto, PriceBookExport, PriceBookPatch, PricingExportPrice},
    support::{DoorError, audit, check_version, conflict, date, invalid, missing, response, value},
};
use crate::{
    domain::book,
    infra::storage::{
        entity::price_book,
        repo::{book_repo, idempotency_repo as idem, price_repo, row_repo},
    },
};
use axum::{http::StatusCode, response::Response};
use toolkit_canonical_errors::CanonicalError;
use toolkit_db::secure::{AccessScope, DBRunner};
use toolkit_security::SecurityContext;
use uuid::Uuid;
pub async fn find(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<price_book::Model, DoorError> {
    book_repo::find(tx, scope, tenant, id)
        .await?
        .ok_or_else(|| missing().into())
}
fn validate(m: &price_book::Model) -> Result<(), CanonicalError> {
    if m.code.trim().is_empty() {
        return Err(invalid("code", "BOOK_CODE_REQUIRED"));
    }
    let errors = book::validate(&book::Book {
        name: m.name.clone(),
        currency: m.currency.clone(),
        valid_from: m.valid_from,
        valid_until: m.valid_until,
    });
    if let Some(e) = errors.first() {
        return Err(invalid("book", e.code));
    }
    Ok(())
}
#[allow(
    clippy::too_many_arguments,
    reason = "Authorized context, replay identity and input belong to one transaction"
)]
pub async fn create(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    key: &str,
    digest: &[u8],
    body: PriceBookCreate,
) -> Result<Response, DoorError> {
    let tenant = ctx.subject_tenant_id();
    let now = time::OffsetDateTime::now_utc();
    let receipt_scope = AccessScope::for_tenant(tenant);
    let endpoint = "/bss-pricing/v1/price-books";
    match idem::claim_idempotency_key(
        tx,
        &receipt_scope,
        tenant,
        endpoint,
        key,
        digest,
        now,
        now + time::Duration::hours(24),
    )
    .await?
    {
        idem::IdempotencyClaim::Claimed => {}
        idem::IdempotencyClaim::Answered {
            payload_hash,
            response_status,
            response_body,
        } => {
            if payload_hash != digest {
                return Err(conflict("IDEMPOTENCY_CONFLICT").into());
            }
            let status = u16::try_from(response_status)
                .ok()
                .and_then(|s| StatusCode::from_u16(s).ok())
                .ok_or_else(|| CanonicalError::internal("invalid stored status").create())?;
            return Ok(response(
                status,
                &response_body,
                response_body["version"].as_u64(),
            )?);
        }
        idem::IdempotencyClaim::InFlight { payload_hash, .. } => {
            return Err(conflict(if payload_hash == digest {
                "IDEMPOTENCY_KEY_IN_FLIGHT"
            } else {
                "IDEMPOTENCY_CONFLICT"
            })
            .into());
        }
        idem::IdempotencyClaim::TakeoverRaceLost => {
            return Err(conflict("IDEMPOTENCY_KEY_IN_FLIGHT").into());
        }
    }
    let model = price_book::Model {
        id: Uuid::now_v7(),
        tenant_id: tenant,
        code: body.code,
        name: body.name,
        currency: body.currency,
        valid_from: date(body.valid_from, "valid_from")?,
        valid_until: date(body.valid_until, "valid_until")?,
        version: 1,
        created_at: now,
        updated_at: now,
    };
    validate(&model)?;
    let model = book_repo::insert(tx, scope, model).await?;
    audit(tx, ctx, correlation, "price_book.create", model.id, 1).await?;
    let body = value(&PriceBookDto::from(model))?;
    if idem::answer_idempotency_key(tx, &receipt_scope, tenant, endpoint, key, 201, body.clone())
        .await?
        != idem::IdempotencyAnswer::Recorded
    {
        return Err(CanonicalError::internal("idempotency claim lost")
            .create()
            .into());
    }
    Ok(response(StatusCode::CREATED, &body, Some(1))?)
}
#[allow(
    clippy::too_many_arguments,
    reason = "Conditional resource identity and audit context are explicit"
)]
pub async fn patch(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    id: Uuid,
    version: u64,
    body: PriceBookPatch,
) -> Result<Response, DoorError> {
    let mut m = find(tx, scope, ctx.subject_tenant_id(), id).await?;
    check_version(version, m.version)?;
    if let Some(name) = body.name {
        m.name = name;
    }
    if let Some(from) = body.valid_from {
        m.valid_from = date(from, "valid_from")?;
    }
    if let Some(until) = body.valid_until {
        m.valid_until = date(until, "valid_until")?;
    }
    validate(&m)?;
    m.updated_at = time::OffsetDateTime::now_utc();
    book_repo::update(tx, scope, m.clone()).await?;
    m.version += 1;
    audit(tx, ctx, correlation, "price_book.update", id, m.version).await?;
    Ok(response(
        StatusCode::OK,
        &PriceBookDto::from(m),
        Some(version + 1),
    )?)
}
pub async fn prices(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<Vec<crate::infra::storage::entity::price::Model>, DoorError> {
    find(tx, &AccessScope::for_tenant(tenant), tenant, id).await?;
    let mut prices = price_repo::for_book(tx, scope, tenant, id).await?;
    prices.sort_by(|a, b| {
        (
            a.sku_id,
            &a.charge_kind,
            a.period.as_deref().unwrap_or(""),
            a.id,
        )
            .cmp(&(
                b.sku_id,
                &b.charge_kind,
                b.period.as_deref().unwrap_or(""),
                b.id,
            ))
    });
    Ok(prices)
}
pub async fn export(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<PriceBookExport, DoorError> {
    let book = find(tx, scope, tenant, id).await?;
    let mut result = Vec::new();
    // The authorized book is the export aggregate; subordinate IDs are not book IDs.
    let children = AccessScope::for_tenant(tenant);
    for p in prices(tx, &children, tenant, id).await? {
        let mut rows = row_repo::for_price(tx, &children, tenant, p.id).await?;
        rows.sort_by(|a, b| {
            (&a.dim_value, a.effective_from, a.version_no, a.id).cmp(&(
                &b.dim_value,
                b.effective_from,
                b.version_no,
                b.id,
            ))
        });
        result.push(PricingExportPrice {
            price: p.into(),
            rows: rows.into_iter().map(Into::into).collect(),
        });
    }
    Ok(PriceBookExport {
        book: book.into(),
        prices: result,
    })
}
