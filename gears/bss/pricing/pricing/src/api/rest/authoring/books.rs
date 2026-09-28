//! Book writes share one transaction with their audit and POST receipt; a book carries an
//! optional description and an unused one is deleted (D-444); the book reads carry their stats
//! (D-441) and the list pages on the toolkit's `OData` pager (D-442).
//!
//! @cpt-dod:cpt-cf-bss-pricing-dod-book-currency-validity:p1
//! @cpt-dod:cpt-cf-bss-pricing-dod-book-export:p1
use super::{
    dto::{
        PriceBookCreate, PriceBookDto, PriceBookExport, PriceBookPatch, PricingExportEntry,
        PricingPriceBookReadDto, PricingPriceDto,
    },
    support::{DoorError, audit, check_version, conflict, date, invalid, missing, response, value},
};
use crate::{
    domain::book,
    infra::storage::{
        entity::price_book,
        repo::{
            book_repo, idempotency_repo as idem, plan_revision_repo, price_book_entry_repo,
            price_repo,
        },
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
    book::validate_description(m.description.as_deref()).map_err(|e| invalid("description", e.code))
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
        description: body.description,
        version: 1,
        created_at: now,
        updated_at: now,
    };
    validate(&model)?;
    // D-438: a new book takes a currency the tenant offers (any, while it offers none).
    super::configuration::offer_currency(tx, tenant, &model.currency).await?;
    let model = book_repo::insert(tx, scope, model).await?;
    audit(tx, ctx, correlation, "price_book.create", model.id, 1).await?;
    let body = value(&PriceBookDto::from(model))?;
    if idem::answer_idempotency_key(
        tx,
        &receipt_scope,
        tenant,
        endpoint,
        key,
        201,
        body.clone(),
        None,
    )
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
    if let Some(description) = body.description {
        m.description = description;
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
/// `DELETE /price-books/{id}` (D-444): an unused book at the version the caller read, with an
/// audit row. Refused in this order, after the door's authorization and If-Match: 404 for a book
/// the tenant does not hold; 409 `STALE_REVISION`; 409 `BOOK_HAS_ENTRIES` for an entry of any
/// reference state; 409 `BOOK_IN_PLAN` for a plan with a draft, pending or published revision on
/// it (`plan_revision_repo::plans_on_books`, the read `stats.plans` counts, D-441); 409
/// `BOOK_IN_PLAN_HISTORY` when only superseded revisions name it (their history keeps the book,
/// and `stats.plans` is 0). No unit can be pending on a book without entries (a pending price
/// keeps its entry), so there is no refusal of its own for one. A row a concurrent writer adds
/// after these reads is the same 409, from the book's foreign key (`book_repo::delete`). Units
/// that named the book stay, and their cards answer without it.
/// # Errors
/// The refusals above; storage failures.
#[allow(
    clippy::too_many_arguments,
    reason = "Conditional resource identity and audit context are explicit"
)]
pub async fn delete(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    backend: sea_orm::DbBackend,
    id: Uuid,
    version: u64,
) -> Result<Response, DoorError> {
    let tenant = ctx.subject_tenant_id();
    let m = find(tx, scope, tenant, id).await?;
    check_version(version, m.version)?;
    let entries = price_book_entry_repo::count_by_book(tx, tenant, backend, &[id]).await?;
    if !entries.is_empty() {
        return Err(conflict("BOOK_HAS_ENTRIES").into());
    }
    if !plan_revision_repo::plans_on_books(tx, tenant, &[id])
        .await?
        .is_empty()
    {
        return Err(conflict("BOOK_IN_PLAN").into());
    }
    if plan_revision_repo::names_book(tx, tenant, id).await? {
        return Err(conflict("BOOK_IN_PLAN_HISTORY").into());
    }
    book_repo::delete(tx, scope, tenant, id, m.version).await?;
    audit(tx, ctx, correlation, "price_book.delete", id, m.version).await?;
    Ok(axum::response::IntoResponse::into_response(
        StatusCode::NO_CONTENT,
    ))
}
/// Each of the tenant's `books` with its stats dated on `today` (D-441), in their order: four
/// grouped statements whatever their number.
/// # Errors
/// Storage failures and corrupt rows.
pub async fn with_stats(
    tx: &impl DBRunner,
    tenant: Uuid,
    backend: sea_orm::DbBackend,
    books: Vec<price_book::Model>,
    today: time::Date,
) -> Result<Vec<PricingPriceBookReadDto>, DoorError> {
    let mut stats = crate::infra::book_stats::book_stats(
        tx,
        tenant,
        backend,
        &books.iter().collect::<Vec<_>>(),
        today,
    )
    .await?;
    books
        .into_iter()
        .map(|b| {
            let counted = stats.remove(&b.id).ok_or_else(|| {
                CanonicalError::internal(format!("no stats for book {}", b.id)).create()
            })?;
            Ok(PricingPriceBookReadDto {
                book: b.into(),
                stats: counted.into(),
            })
        })
        .collect()
}
/// `GET /price-books` (D-442): one page of the tenant's books under the caller's `scope`,
/// narrowed by `filter` and the query, each with its stats (D-441): the page's statement and the
/// stats' four.
/// # Errors
/// 400 for a query the pager refuses; storage failures and corrupt rows.
pub async fn page(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    backend: sea_orm::DbBackend,
    filter: &book_repo::BookListFilter,
    query: &toolkit_odata::ODataQuery,
    today: time::Date,
) -> Result<toolkit_odata::Page<PricingPriceBookReadDto>, DoorError> {
    let page = book_repo::page(tx, scope, tenant, backend, filter, query)
        .await
        .map_err(|e| match e {
            book_repo::BookListError::Query(e) => DoorError::Api(e.into()),
            book_repo::BookListError::Repo(e) => DoorError::Repo(e),
        })?;
    Ok(toolkit_odata::Page {
        items: with_stats(tx, tenant, backend, page.items, today).await?,
        page_info: page.page_info,
    })
}
pub async fn entries(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<Vec<crate::infra::storage::entity::price_book_entry::Model>, DoorError> {
    find(tx, &AccessScope::for_tenant(tenant), tenant, id).await?;
    let mut entries = price_book_entry_repo::for_book(tx, scope, tenant, id).await?;
    entries.sort_by(|a, b| {
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
    Ok(entries)
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
    for p in entries(tx, &children, tenant, id).await? {
        let mut prices = price_repo::for_entry(tx, &children, tenant, p.id).await?;
        prices.sort_by(|a, b| {
            (&a.dim_value, a.effective_from, a.version_no, a.id).cmp(&(
                &b.dim_value,
                b.effective_from,
                b.version_no,
                b.id,
            ))
        });
        // Every price echoes its entry's model (D-427).
        let model = p.model.clone();
        result.push(PricingExportEntry {
            entry: p.try_into()?,
            prices: prices
                .into_iter()
                .map(|m| PricingPriceDto::of(m, &model))
                .collect::<Result<_, _>>()?,
        });
    }
    Ok(PriceBookExport {
        book: book.into(),
        entries: result,
    })
}
