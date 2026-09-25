//! Price book entry writes and their durable registry operations.
//!
//! @cpt-dod:cpt-cf-bss-pricing-dod-entry-key-unique:p1
//! @cpt-dod:cpt-cf-bss-pricing-dod-entry-metadata:p1
//! @cpt-dod:cpt-cf-bss-pricing-dod-entry-reference-handoff:p1
use super::{
    AuthoringState,
    dto::{PricingPriceBookEntryCreate, PricingPriceBookEntryDto, PricingPriceBookEntryPatch},
    support::{self, DoorError},
};
use crate::{
    domain::{
        price::PriceState,
        price_book_entry::{self, OpKind},
    },
    infra::{
        reference_work::{self, Caller, Receipt, WallClock, Work},
        storage::{
            entity,
            repo::{
                book_repo, dimension_repo, idempotency_repo as idem, price_book_entry_repo,
                price_repo, reference_op_repo,
            },
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
) -> Result<entity::price_book_entry::Model, DoorError> {
    price_book_entry_repo::find(tx, scope, tenant, id)
        .await?
        .ok_or_else(|| support::missing_entry().into())
}
fn validate_template(input: Option<&str>) -> Result<(), CanonicalError> {
    if let Some(template) = input {
        price_book_entry::validate_template(template)
            .map_err(|e| support::invalid("invoice_line_override", e.code))?;
    }
    Ok(())
}
enum Begun {
    Replay(Receipt),
    Op(Uuid),
}
/// What a held Idempotency-Key answers: `None` when this call holds it (or may take it).
fn settled(
    claim: idem::IdempotencyClaim,
    digest: &[u8],
) -> Result<Option<Receipt>, CanonicalError> {
    match claim {
        idem::IdempotencyClaim::Claimed => Ok(None),
        idem::IdempotencyClaim::Answered {
            payload_hash,
            response_body,
            ..
        } => {
            if payload_hash != digest {
                return Err(support::conflict("IDEMPOTENCY_CONFLICT"));
            }
            serde_json::from_value(response_body)
                .map(Some)
                .map_err(|_| CanonicalError::internal("invalid entry receipt").create())
        }
        idem::IdempotencyClaim::InFlight { payload_hash, .. } if payload_hash != digest => {
            Err(support::conflict("IDEMPOTENCY_CONFLICT"))
        }
        _ => Err(support::conflict("IDEMPOTENCY_KEY_IN_FLIGHT")),
    }
}
/// The key's stored answer, read without claiming it: a replay or an in-flight duplicate is
/// answered from the store alone, before any Products call.
async fn stored(
    state: &AuthoringState,
    tenant: Uuid,
    endpoint: &str,
    key: &str,
    digest: &[u8],
) -> Result<Option<Receipt>, CanonicalError> {
    let conn = state.db.conn().map_err(DoorError::from)?;
    match idem::lookup_idempotency_key(
        &conn,
        &AccessScope::for_tenant(tenant),
        tenant,
        endpoint,
        key,
        time::OffsetDateTime::now_utc(),
    )
    .await
    .map_err(DoorError::from)?
    {
        Some(claim) => settled(claim, digest),
        None => Ok(None),
    }
}
/// The period rule needs the SKU's type, read before anything is claimed or reserved: an
/// input refusal is 400 and costs no reservation (D-403). A registry that cannot answer is
/// 503 with nothing written; a definite Products refusal is answered as Products gave it.
async fn check_period(
    state: &AuthoringState,
    ctx: &SecurityContext,
    input: &PricingPriceBookEntryCreate,
) -> Result<(), CanonicalError> {
    let registry = crate::infra::reference_registry::resolve(&state.hub)
        .map_err(|_| support::unavailable())?;
    let sku = registry
        .sku_for_write(ctx, ctx.subject_tenant_id(), input.sku_id)
        .await
        .map_err(|error| {
            if reference_work::definite_refusal(&error) {
                error
            } else {
                support::unavailable()
            }
        })?;
    if price_book_entry::period_valid(sku.r#type, input.period.as_deref()) {
        Ok(())
    } else {
        Err(support::invalid("period", "ENTRY_PERIOD_INVALID"))
    }
}
/// A named dimension key must be declared in the tenant's registry (the seed key counts while
/// the tenant stores none; the entry write stores it).
async fn check_dimension(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    key: Option<&str>,
) -> Result<(), DoorError> {
    if let Some(key) = key
        && !dimension_repo::declared(tx, scope, tenant, key).await?
    {
        return Err(support::invalid("dimension_key", "DIM_NOT_DECLARED").into());
    }
    Ok(())
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
    input: PricingPriceBookEntryCreate,
) -> Result<Response, CanonicalError> {
    let original_ctx = ctx.clone();
    let endpoint = format!("/bss-pricing/v1/price-books/{book}/entries");
    if let Some(receipt) = stored(&state, ctx.subject_tenant_id(), &endpoint, &key, &digest).await?
    {
        return receipt.response();
    }
    check_period(&state, &ctx, &input).await?;
    let result = support::transaction(&state.db.db(), move |tx| {
        let (scope, ctx, key, digest, input, endpoint) = (
            scope.clone(),
            ctx.clone(),
            key.clone(),
            digest.clone(),
            input.clone(),
            endpoint.clone(),
        );
        Box::pin(async move {
            let tenant = ctx.subject_tenant_id();
            let now = time::OffsetDateTime::now_utc();
            let receipt_scope = AccessScope::for_tenant(tenant);
            let claim = idem::claim_idempotency_key(
                tx,
                &receipt_scope,
                tenant,
                &endpoint,
                &key,
                &digest,
                now,
                now + time::Duration::hours(24),
            )
            .await?;
            if let Some(receipt) = settled(claim, &digest)? {
                return Ok(Begun::Replay(receipt));
            }
            validate_template(input.invoice_line_override.as_deref())?;
            check_dimension(tx, &receipt_scope, tenant, input.dimension_key.as_deref()).await?;
            if book_repo::find(tx, &scope, tenant, book).await?.is_none() {
                return Err(support::missing().into());
            }
            let work = Work {
                book_id: book,
                input,
                correlation,
                refusal: None,
                receipt: None,
                outcome: None,
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
        Begun::Op(id) => {
            reference_work::drive(&state, &original_ctx, id, Arc::new(WallClock), Caller::Door)
                .await?
                .ok_or_else(|| CanonicalError::internal("missing create receipt").create())?
                .response()
        }
    }
}
pub(super) async fn patch(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    correlation: Uuid,
    id: Uuid,
    version: u64,
    input: PricingPriceBookEntryPatch,
) -> Result<Response, DoorError> {
    let tenant = ctx.subject_tenant_id();
    let mut m = find(tx, scope, tenant, id).await?;
    support::check_version(version, m.version)?;
    if let Some(dimension) = input.dimension_key {
        check_dimension(tx, scope, tenant, dimension.as_deref()).await?;
        if dimension != m.dimension_key
            && price_repo::for_entry(tx, scope, tenant, id)
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
    price_book_entry_repo::update(tx, scope, m.clone()).await?;
    m.version += 1;
    support::audit(
        tx,
        ctx,
        correlation,
        "price_book_entry.patch",
        id,
        m.version,
    )
    .await?;
    Ok(support::response(
        StatusCode::OK,
        &PricingPriceBookEntryDto::from(m),
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
            // A pending create must complete before deletion, otherwise its confirm could lose its entry.
            if m.reference_state == "confirmation_pending" {
                return Err(support::conflict("ENTRY_CONFIRMATION_PENDING").into());
            }
            // Approved or pending money blocks deletion; drafts and rejected proposals go with
            // the entry (a rejected price's history stays in its unit's snapshot).
            let prices = price_repo::for_entry(tx, &scope, tenant, id).await?;
            if prices.iter().any(|price| {
                !matches!(price.state.as_str(), "draft" | "rejected")
                    || price.pending_unit_id.is_some()
            }) {
                return Err(support::conflict("ENTRY_PRICES_IN_USE").into());
            }
            // D-404: a draft belongs to its author, and deleting the entry would delete it. A
            // rejected price is history (its unit's snapshot keeps it) and never blocks.
            if let Some(foreign) = prices.iter().find(|price| {
                price.state == PriceState::Draft.as_str() && price.created_by != ctx.subject_id()
            }) {
                return Err(support::forbidden_because(
                    "NOT_DRAFT_AUTHOR",
                    format!("price {} is a draft of another author", foreign.id),
                )
                .into());
            }
            let prices: Vec<_> = prices
                .iter()
                .map(|price| (price.id, price.version))
                .collect();
            price_repo::delete_unapproved(tx, &scope, tenant, &prices).await?;
            let work = Work {
                book_id: m.book_id,
                input: PricingPriceBookEntryCreate {
                    sku_id: m.sku_id,
                    period: m.period,
                    dimension_key: m.dimension_key,
                    invoice_line_override: m.invoice_line_override,
                },
                correlation,
                refusal: None,
                receipt: None,
                outcome: None,
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
            price_book_entry_repo::delete_empty(tx, &scope, tenant, id, m.version).await?;
            reference_op_repo::insert(tx, &AccessScope::for_tenant(tenant), op).await?;
            support::audit(
                tx,
                &ctx,
                correlation,
                "price_book_entry.delete",
                id,
                m.version,
            )
            .await?;
            Ok(op_id)
        })
    })
    .await?;
    // The entry is gone once the transaction commits: answer 204. The release is durable work;
    // what this door does not finish, the ticker does.
    if let Err(error) = reference_work::drive(
        &state,
        &original_ctx,
        op_id,
        Arc::new(WallClock),
        Caller::Door,
    )
    .await
    {
        tracing::warn!(op_id=%op_id, error=%error, "pricing entry release deferred to the ticker");
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}
